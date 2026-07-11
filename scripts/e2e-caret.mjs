// e2e-caret.mjs — headless repro for Tine ArrowUp/Down caret bugs.
//
// Bug A: caret vanishes on SOME SCHEDULED/DEADLINE blocks during keyboard nav.
// Bug B: column preservation across blocks (goal-column fix).
//
// Two modes (env CARET_MODE):
//   page    (default) — single CaretTest page, Round-1 fixture (bug B confirm).
//   journal           — multi-day journal FEED with structural variants (bug A).
//   agenda            — release-required duplicate-instance invariant (ADR 0013).
//
// Usage:
//   CARET_MODE=agenda node scripts/e2e-caret.mjs
//   CARET_MODE=journal node scripts/e2e-caret.mjs
//
// Appends findings to subagent-tasks/notes/caret-updown-repro-findings.md.
// Does NOT modify Tine src/.

import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const MODE = process.env.CARET_MODE || "page";
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const G = "/tmp/txdg-caret-g";
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
const LOCAL_WEBKIT_DRIVER = "/tmp/tine-webdriver/usr/bin/WebKitWebDriver";
const WEBKIT_DRIVER =
  process.env.WEBKIT_DRIVER ||
  (fs.existsSync("/usr/bin/WebKitWebDriver")
    ? "/usr/bin/WebKitWebDriver"
    : fs.existsSync(LOCAL_WEBKIT_DRIVER)
      ? LOCAL_WEBKIT_DRIVER
      : "WebKitWebDriver");
const DAY_MS = 24 * 60 * 60 * 1000;
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

function noon(d = new Date()) {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate(), 12, 0, 0, 0);
}

function addDays(d, n) {
  return new Date(d.getTime() + n * DAY_MS);
}

function pad2(n) {
  return String(n).padStart(2, "0");
}

function journalFileStem(d) {
  return `${d.getFullYear()}_${pad2(d.getMonth() + 1)}_${pad2(d.getDate())}`;
}

function logseqDate(d) {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${WEEKDAYS[d.getDay()]}`;
}

let xvfb;
async function ensureDisplay() {
  if (process.env.DISPLAY) return;
  const displays = process.env.XVFB_DISPLAY
    ? [process.env.XVFB_DISPLAY]
    : [":98", ":99", ":100", ":101"];
  let lastLog = "/tmp/xvfb-caret.log";
  let lastError = "";
  for (const display of displays) {
    const suffix = display.replace(/[^0-9]/g, "") || "x";
    lastLog = `/tmp/xvfb-caret-${suffix}.log`;
    const xvfbLog = fs.openSync(lastLog, "w");
    let spawnError = "";
    const child = spawn("Xvfb", [display, "-screen", "0", "1400x1000x24"], {
      stdio: ["ignore", xvfbLog, xvfbLog],
    });
    child.on("error", (e) => {
      spawnError = e.message;
    });
    await sleep(900);
    if (spawnError) {
      lastError = spawnError;
      continue;
    }
    if (child.exitCode == null) {
      xvfb = child;
      process.env.DISPLAY = display;
      return;
    }
    lastError = `display ${display} exited with code ${child.exitCode}`;
  }
  if (lastError) {
    throw new Error(`Xvfb failed to start (${lastError}); see ${lastLog}`);
  }
}

// ---- Fixtures ----------------------------------------------------------------
const PAGE_FIXTURE = `- TODO alpha one two three four
- TODO beta five six
- TODO gamma seven eight
  DEADLINE: <2026-06-26 Fri>
- TODO delta nine ten eleven
- TODO epsilon twelve
  SCHEDULED: <2026-07-01 Wed>
- TODO zeta thirteen fourteen
- TODO eta fifteen
  DEADLINE: <2026-07-31 Fri>
- TODO theta the end
`;

// Journal feed fixtures. Structural variants (the "some I can access, some I
// can't" suspects) are spread across days:
//   (a) planning block + hidden id::  (07_01 beta): editorValue != raw.length
//   (b) planning block nested as a CHILD (07_01 delta-child)
//   (c) planning child under a COLLAPSED parent (06_30 iota) — must skip past it
//   (d) planning block that is LAST block of a day (07_01 epsilon; day boundary)
//   (e) two consecutive planning blocks (06_30 eta + theta)
//   (f) referenced planning block (id::) reached via Up from below (06_29 mu)
const J_0701 = [
  "- TODO alpha morning task",
  "- TODO beta with deadline",
  "  DEADLINE: <2026-07-02 Thu>",
  "  id:: 6650aaaa-bbbb-cccc-dddd-000000000001",
  "- TODO gamma plain middle",
  "- TODO delta parent block",
  "\t- TODO delta-child scheduled",
  "\t  SCHEDULED: <2026-07-03 Fri>",
  "- TODO epsilon last of day",
  "  DEADLINE: <2026-07-05 Sun>",
  "",
].join("\n");

const J_0630 = [
  "- TODO zeta first task",
  "- TODO eta consec planning one",
  "  DEADLINE: <2026-07-01 Wed>",
  "- TODO theta consec planning two",
  "  SCHEDULED: <2026-07-02 Thu>",
  "- TODO iota collapsed parent",
  "  collapsed:: true",
  "\t- TODO iota-child under collapsed",
  "\t  DEADLINE: <2026-07-04 Sat>",
  "- TODO kappa after collapsed",
  "",
].join("\n");

const J_0629 = [
  "- TODO lambda plain start",
  "- TODO mu referenced planning",
  "  DEADLINE: <2026-07-01 Wed>",
  "  id:: 6650aaaa-bbbb-cccc-dddd-000000000002",
  "- TODO nu below referenced",
  "- TODO xi end of feed day",
  "",
].join("\n");

const J_0628 = [
  "- TODO omicron deep day first",
  "- TODO pi deep planning",
  "  SCHEDULED: <2026-07-06 Mon>",
  "- TODO rho deep last",
  "",
].join("\n");

function agendaFixture(today = noon()) {
  return [
    "- TODO alpha one two three",
    "- TODO beta four five",
    "- TODO gamma six seven",
    `  SCHEDULED: <${logseqDate(today)}>`,
    "- TODO delta eight",
    `  DEADLINE: <${logseqDate(addDays(today, 2))}>`,
    "- TODO epsilon nine ten",
    "- TODO zeta scheduled soon",
    `  SCHEDULED: <${logseqDate(addDays(today, -1))}>`,
    "- TODO eta deadline soon",
    `  DEADLINE: <${logseqDate(addDays(today, 3))}>`,
    "",
  ].join("\n");
}

function seed() {
  fs.rmSync(G, { recursive: true, force: true });
  fs.mkdirSync(`${G}/pages`, { recursive: true });
  fs.mkdirSync(`${G}/journals`, { recursive: true });
  fs.mkdirSync(`${G}/logseq`, { recursive: true });
  if (MODE === "agenda") {
    const today = noon();
    fs.writeFileSync(`${G}/journals/${journalFileStem(today)}.md`, agendaFixture(today));
  } else if (MODE === "journal") {
    fs.writeFileSync(`${G}/journals/2026_07_01.md`, J_0701);
    fs.writeFileSync(`${G}/journals/2026_06_30.md`, J_0630);
    fs.writeFileSync(`${G}/journals/2026_06_29.md`, J_0629);
    fs.writeFileSync(`${G}/journals/2026_06_28.md`, J_0628);
  } else {
    fs.writeFileSync(`${G}/pages/CaretTest.md`, PAGE_FIXTURE);
    fs.writeFileSync(`${G}/journals/2026_07_01.md`, "- open [[CaretTest]]\n");
  }
}

seed();
await ensureDisplay();

fs.rmSync("/tmp/txdg", { recursive: true, force: true });
for (const d of ["data", "config", "cache"])
  fs.mkdirSync(`/tmp/txdg/${d}`, { recursive: true });

const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/txdg/data",
  XDG_CONFIG_HOME: "/tmp/txdg/config",
  XDG_CACHE_HOME: "/tmp/txdg/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};
console.log(
  "DISPLAY=", process.env.DISPLAY,
  "MODE=", MODE,
  "APP=", APP,
  "WEBKIT_DRIVER=", WEBKIT_DRIVER
);

const tdLog = fs.openSync("/tmp/td-caret.log", "w");
const td = spawn(
  TD,
  ["--port", "4444", "--native-port", "4445", "--native-driver", WEBKIT_DRIVER],
  { env, stdio: ["ignore", tdLog, tdLog] }
);
await sleep(3000);

const lines = [];
const log = (line) => {
  console.log(line);
  lines.push(line);
};

let traceRows = [];
const probe = async (tag) => {
  const s = await browser.execute(() => {
    const ae = document.activeElement;
    const isEd =
      ae instanceof HTMLTextAreaElement && ae.classList.contains("block-editor");
    const blocks = [...document.querySelectorAll(".ls-block")];
    const closest = isEd && ae.closest ? ae.closest(".ls-block") : null;
    const idx = closest ? blocks.indexOf(closest) : -1;
    // App's own "which block is editing" = the .block-main carrying class `editing`.
    const editingMain = document.querySelector(".block-main.editing");
    const editingBlock = editingMain ? editingMain.closest(".ls-block") : null;
    const editingId = editingBlock ? editingBlock.getAttribute("data-block-id") : null;
    const editingIdx = editingBlock ? blocks.indexOf(editingBlock) : -1;
    // Does the currently-editing block still show a deferred (lazy) body?
    const editingDeferred = editingBlock
      ? !!editingBlock.querySelector(":scope > .block-main .ast-deferred")
      : null;
    const focusDeferred = closest
      ? !!closest.querySelector(":scope > .block-main .ast-deferred")
      : null;
    return {
      isEditor: isEd,
      sel: isEd ? ae.selectionStart : null,
      val: isEd ? ae.value.slice(0, 34) : null,
      idx,
      editingId: editingId ? editingId.slice(0, 12) : null,
      editingIdx,
      editingDeferred,
      focusDeferred,
      aeTag: ae ? ae.tagName : "null",
      aeCls: ae ? String(ae.className).slice(0, 40) : "",
      nblocks: blocks.length,
    };
  });
  const status = s.isEditor ? "OK        " : "CARET_LOST";
  const line =
    `  ${tag.padEnd(16)} ${status} sel=${String(s.sel).padStart(3)} idx=${String(s.idx).padStart(2)}` +
    ` eId=${String(s.editingId).padEnd(12)} eIdx=${String(s.editingIdx).padStart(2)}` +
    ` def=${String(s.focusDeferred)}/${String(s.editingDeferred)} val=${JSON.stringify(s.val)}`;
  log(line);
  traceRows.push({ tag, ...s });
  return s;
};

// Dump the full block map (idx → text, deferred, block-id, collapsed).
const dumpBlockMap = async (label) => {
  const map = await browser.execute(() => {
    const blocks = [...document.querySelectorAll(".ls-block")];
    return blocks.map((b, i) => {
      const main = b.querySelector(":scope > .block-main");
      const deferred = main ? !!main.querySelector(".ast-deferred") : false;
      const ta = b.querySelector(":scope > .block-main textarea.block-editor");
      let text = "";
      if (ta) text = ta.value.slice(0, 40);
      else {
        const body = main ? main.querySelector(".ast-body, .ast-fallback, .block-content") : null;
        text = (body ? body.textContent : b.textContent || "").trim().slice(0, 40);
      }
      return {
        i,
        id: (b.getAttribute("data-block-id") || "").slice(0, 12),
        deferred,
        collapsed: b.classList.contains("collapsed"),
        text,
      };
    });
  });
  log(`\n--- BLOCK MAP (${label}) — ${map.length} .ls-block rows ---`);
  for (const m of map)
    log(
      `  [${String(m.i).padStart(2)}] ${m.deferred ? "DEFER" : "     "} ${m.collapsed ? "COLL" : "    "} id=${m.id.padEnd(12)} ${JSON.stringify(m.text)}`
    );
  return map;
};

const clickBlock = async (blockIdx, selStart) => {
  await browser.execute((idx) => {
    const blocks = [...document.querySelectorAll(".ls-block")];
    const block = blocks[idx];
    if (!block) return;
    const wrapper = block.querySelector(":scope > .block-main .block-content-wrapper");
    // Edit entry is a mousedown-armed / mouseup-resolved gesture (Jul 2 2026) —
    // element.click() fires neither, so dispatch the down+up pair (same point =
    // a click, which starts editing).
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
  await sleep(120);
};

// ---- Round 3: agenda-duplicate BEFORE/AFTER scenario ------------------------
async function runAgendaScenario() {
  const targetText = "gamma six seven";
  const aboveText = "beta four five";
  const typed = "q";
  const failures = [];
  const expect = (cond, msg) => {
    if (!cond) {
      failures.push(msg);
      log(`    !! ${msg}`);
    }
  };
  const lineStart = (value, pos) => value.slice(0, pos).lastIndexOf("\n") + 1;
  const firstLineLen = (value) => {
    const nl = value.indexOf("\n");
    return nl === -1 ? value.length : nl;
  };
  const expectedDownSel = (fromValue, fromSel, toValue) => {
    const col = fromSel - lineStart(fromValue, fromSel);
    return Math.min(col, firstLineLen(toValue));
  };
  const expectedUpSel = (fromValue, fromSel, toValue) => {
    const col = fromSel - lineStart(fromValue, fromSel);
    const start = toValue.lastIndexOf("\n") + 1;
    return start + Math.min(col, toValue.length - start);
  };

  const state = async (targetId = null, aboveId = null) =>
    browser.execute(
      (targetTextArg, aboveTextArg, targetIdArg, aboveIdArg) => {
        const inAgenda = (el) => !!el.closest(".agenda-block, .query-block");
        const textOf = (b) => {
          const ta = b.querySelector(":scope > .block-main textarea.block-editor");
          return ta ? ta.value : (b.textContent || "");
        };
        const blockInfo = (b) => {
          const ta = b.querySelector(":scope > .block-main textarea.block-editor");
          const main = b.querySelector(":scope > .block-main");
          return {
            id: b.getAttribute("data-block-id"),
            inAgenda: inAgenda(b),
            text: textOf(b).trim(),
            hasEditor: !!ta,
            editing: !!main?.classList.contains("editing"),
          };
        };
        const blocks = [...document.querySelectorAll(".ls-block")];
        const byText = (t) => blocks.filter((b) => textOf(b).includes(t)).map(blockInfo);
        const byId = (id) =>
          id ? blocks.filter((b) => b.getAttribute("data-block-id") === id).map(blockInfo) : [];
        const editors = [...document.querySelectorAll("textarea.block-editor")].map((ta) => {
          const block = ta.closest(".ls-block");
          return {
            id: block ? block.getAttribute("data-block-id") : null,
            inAgenda: block ? inAgenda(block) : false,
            active: ta === document.activeElement,
            sel: ta.selectionStart,
            value: ta.value,
          };
        });
        const editingMains = [...document.querySelectorAll(".block-main.editing")].map((main) => {
          const block = main.closest(".ls-block");
          return {
            id: block ? block.getAttribute("data-block-id") : null,
            inAgenda: block ? inAgenda(block) : false,
          };
        });
        const active = document.activeElement;
        const activeEditor =
          active instanceof HTMLTextAreaElement && active.classList.contains("block-editor")
            ? editors.find((e) => e.active) ?? null
            : null;
        return {
          hasAgenda: !!document.querySelector(".agenda-block"),
          agendaCopies: document.querySelectorAll(".agenda-block .ls-block, .query-block .ls-block").length,
          targetByText: byText(targetTextArg),
          aboveByText: byText(aboveTextArg),
          targetById: byId(targetIdArg),
          aboveById: byId(aboveIdArg),
          editors,
          editingMains,
          activeEditor,
          activeTag: active ? active.tagName : "null",
          activeClass: active ? String(active.className) : "",
          scrollTop: Math.round(document.querySelector(".main-content")?.scrollTop ?? -1),
        };
      },
      targetText,
      aboveText,
      targetId,
      aboveId
    );

  const waitForDuplicate = async () => {
    let last = null;
    await browser.waitUntil(
      async () => {
        last = await state();
        const feedTarget = last.targetByText.find((b) => !b.inAgenda);
        const agendaTarget = last.targetByText.find((b) => b.inAgenda);
        const feedAbove = last.aboveByText.find((b) => !b.inAgenda);
        return !!last.hasAgenda && !!feedTarget && !!agendaTarget && feedTarget.id === agendaTarget.id && !!feedAbove;
      },
      { timeout: 15000, interval: 250, timeoutMsg: "agenda duplicate target did not render" }
    );
    return last;
  };

  const fixture = await waitForDuplicate();
  const feedTarget = fixture.targetByText.find((b) => !b.inAgenda);
  const agendaTarget = fixture.targetByText.find((b) => b.inAgenda);
  const feedAbove = fixture.aboveByText.find((b) => !b.inAgenda);
  const targetId = feedTarget.id;
  const aboveId = feedAbove.id;

  log(`Agenda present: ${fixture.hasAgenda}   agenda .ls-block copies: ${fixture.agendaCopies}`);
  log(
    `DOM duplicate target: id=${String(targetId).slice(0, 12)} feed=${!!feedTarget} agenda=${!!agendaTarget}` +
      ` occurrences=${fixture.targetByText.length}`
  );

  const clickFeedBlock = async (id, caret) => {
    await browser.execute(
      (blockId) => {
        const blocks = [...document.querySelectorAll(".ls-block")];
        const block = blocks.find(
          (b) =>
            b.getAttribute("data-block-id") === blockId &&
            !b.closest(".agenda-block, .query-block")
        );
        if (!block) return false;
        const wrapper = block.querySelector(":scope > .block-main .block-content-wrapper");
        // Edit entry is a mousedown-armed / mouseup-resolved gesture (Jul 2 2026) —
        // element.click() fires neither, so dispatch the down+up pair (same point =
        // a click, which starts editing).
        const el = wrapper || block;
        el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
        el.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
        return true;
      },
      id
    );
    await sleep(450);
    await browser.execute(
      (wantedCaret) => {
        const ae = document.activeElement;
        if (!(ae instanceof HTMLTextAreaElement)) return false;
        const pos = wantedCaret === "end" ? ae.value.length : Number(wantedCaret);
        ae.setSelectionRange(pos, pos);
        return true;
      },
      caret === "end" ? "end" : String(caret)
    );
    await sleep(150);
  };

  const assertAgendaCopyRendered = (s, tag) => {
    const agendaCopy = s.targetById.find((b) => b.inAgenda);
    expect(!!agendaCopy, `${tag}: agenda copy for target uuid is missing`);
    if (!agendaCopy) return;
    expect(!agendaCopy.hasEditor, `${tag}: agenda copy mounted an editor`);
    expect(agendaCopy.text.length > 0, `${tag}: agenda copy rendered blank`);
  };

  const assertFocusedEditor = (s, tag, expectedId, expectedSel, requireDuplicate) => {
    log(
      `    ${tag.padEnd(18)} active=${s.activeEditor ? "editor" : s.activeTag}` +
        ` editors=${s.editors.length} editing=${s.editingMains.length}` +
        ` id=${String(s.activeEditor?.id ?? "").slice(0, 12)} inAgenda=${s.activeEditor?.inAgenda}` +
        ` sel=${s.activeEditor?.sel} scrollTop=${s.scrollTop}` +
        ` val=${JSON.stringify((s.activeEditor?.value ?? "").slice(0, 42))}`
    );
    expect(!!s.activeEditor, `${tag}: activeElement is not textarea.block-editor`);
    expect(s.editors.length === 1, `${tag}: expected exactly one mounted block editor, saw ${s.editors.length}`);
    expect(s.editingMains.length === 1, `${tag}: expected exactly one .block-main.editing, saw ${s.editingMains.length}`);
    expect(s.activeEditor?.id === expectedId, `${tag}: active editor id ${s.activeEditor?.id} != ${expectedId}`);
    expect(s.activeEditor?.inAgenda === false, `${tag}: active editor is in the agenda/ref surface`);
    expect(s.activeEditor?.sel === expectedSel, `${tag}: caret ${s.activeEditor?.sel} != expected ${expectedSel}`);
    if (requireDuplicate) {
      expect(s.targetById.length === 2, `${tag}: expected two rendered target instances, saw ${s.targetById.length}`);
      assertAgendaCopyRendered(s, tag);
    }
  };

  log(`\n  CASE: ADR 0013 duplicate-instance invariant`);
  await clickFeedBlock(aboveId, "end");
  const clicked = await state(targetId, aboveId);
  const beforeEditor = clicked.activeEditor;
  expect(!!beforeEditor, "click(start): failed to focus the block above the scheduled task");
  const beforeValue = beforeEditor?.value ?? "";
  const beforeSel = beforeEditor?.sel ?? 0;
  assertFocusedEditor(clicked, "click(start)", aboveId, beforeSel, false);
  assertAgendaCopyRendered(clicked, "click(start)");

  await browser.keys(["ArrowDown"]);
  await sleep(550);
  const down = await state(targetId, aboveId);
  const targetValueBeforeType = down.activeEditor?.value ?? "";
  const downSel = expectedDownSel(beforeValue, beforeSel, targetValueBeforeType);
  assertFocusedEditor(down, "ArrowDown", targetId, downSel, true);

  await browser.keys([typed]);
  await sleep(250);
  const typedState = await state(targetId, aboveId);
  const expectedTypedValue =
    targetValueBeforeType.slice(0, downSel) + typed + targetValueBeforeType.slice(downSel);
  assertFocusedEditor(typedState, "type one char", targetId, downSel + 1, true);
  expect(
    typedState.activeEditor?.value === expectedTypedValue,
    "type one char: inserted character did not land in the scheduled feed block"
  );

  await browser.keys(["ArrowUp"]);
  await sleep(550);
  const up = await state(targetId, aboveId);
  const aboveValue = up.activeEditor?.value ?? "";
  const upSel = expectedUpSel(expectedTypedValue, downSel + 1, aboveValue);
  assertFocusedEditor(up, "ArrowUp", aboveId, upSel, false);
  assertAgendaCopyRendered(up, "ArrowUp");

  const anyBug = failures.length > 0;
  log(`\n  --- AGENDA INVARIANT SUMMARY (${process.env.CARET_LABEL || "?"}) ---`);
  if (anyBug) {
    for (const f of failures) log(`    FAIL ${f}`);
  } else {
    log("    PASS one feed editor, agenda copy rendered, deterministic caret, typed char landed");
  }
  log(`  VERDICT: ${anyBug ? "BUG PRESENT (ADR 0013 invariant failed)" : "CLEAN (ADR 0013 invariant holds)"}`);
  return { hasAgenda: fixture.hasAgenda, copies: fixture.agendaCopies, failures, anyBug };
}

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: 4444,
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

  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
  await sleep(1500);

  if (MODE === "page") {
    // ---- PART 1: single CaretTest page, confirm goal-column fix (bug B) -------
    log("\n=== PAGE MODE: navigate to CaretTest ===");
    for (const sel of ["a.page-ref=CaretTest", "span.page-ref=CaretTest", "*=CaretTest"]) {
      const el = await browser.$(sel);
      if (await el.isExisting()) { await el.click(); log(`opened via: ${sel}`); break; }
    }
    await sleep(2000);
  } else {
    // ---- PART 2: journal feed. The app opens journals by default. -------------
    log("\n=== JOURNAL MODE: journals feed ===");
    const titles = await browser.execute(
      () => document.querySelectorAll(".page-title, .journal-title, .journal-day").length
    );
    log(`page/journal title sections visible: ${titles}`);
    await sleep(1500);
  }

  const chipCount = await browser.execute(
    () => document.querySelectorAll(".date-chip").length
  );
  const blockCount = await browser.execute(
    () => document.querySelectorAll(".ls-block").length
  );
  log(`Blocks: ${blockCount}   date-chips: ${chipCount}`);

  if (MODE === "agenda") {
   const agenda = await runAgendaScenario();
   if (agenda.anyBug) {
     throw new Error(`agenda duplicate invariant failed (${agenda.failures.length} failure(s))`);
   }
  } else {
  await dumpBlockMap("initial");

  const runSweep = async (label, startIdx, startOffset, nDown, nUp, upFromIdx) => {
    log(`\n${"=".repeat(64)}`);
    log(`SWEEP: ${label}  (start idx=${startIdx} offset=${startOffset})`);
    log("=".repeat(64));
    traceRows = [];

    await clickBlock(startIdx, startOffset);
    await probe(`INIT-${startIdx}`);

    log("--- ArrowDown ---");
    let lastSeen = startIdx;
    for (let i = 1; i <= nDown; i++) {
      await browser.keys(["ArrowDown"]);
      await sleep(420);
      const s = await probe(`↓[${i}]`);
      if (s.isEditor) lastSeen = s.idx;
    }
    log(`Last reachable idx via Down: ${lastSeen}`);

    const upIdx = upFromIdx != null ? upFromIdx : lastSeen;
    await sleep(300);
    await clickBlock(upIdx, startOffset === 0 ? 0 : "end");
    await probe(`INIT-${upIdx}`);
    log("--- ArrowUp ---");
    for (let i = 1; i <= nUp; i++) {
      await browser.keys(["ArrowUp"]);
      await sleep(420);
      await probe(`↑[${i}]`);
    }

    const lost = traceRows.filter((r) => !r.isEditor && !r.tag.startsWith("INIT"));
    if (lost.length === 0) log(`  >> No caret losses in this sweep.`);
    else {
      log(`  >> CARET LOSSES (${lost.length}):`);
      for (const r of lost)
        log(`       ${r.tag}: editingId=${r.editingId} editingIdx=${r.editingIdx} focusDeferred=${r.focusDeferred} ae=${r.aeTag}.${r.aeCls}`);
    }
    return { label, trace: [...traceRows], losses: [...lost] };
  };

  const results = [];

  if (MODE === "page") {
    results.push(await runSweep("PAGE col-7 down/up", 0, 7, 15, 15));
  } else {
    results.push(await runSweep("JOURNAL col-7 full sweep", 0, 7, 30, 30));
    await sleep(400);
    results.push(await runSweep("JOURNAL col-0 full sweep", 0, 0, 30, 30));
    await sleep(400);
    results.push(await runSweep("JOURNAL end-of-line full sweep", 0, "end", 30, 30));

    // Explicit user action: click a PLAIN todo, then ArrowDown into the block below.
    log(`\n${"=".repeat(64)}`);
    log("USER-ACTION: click a plain TODO, press ArrowDown into the block below");
    log("=".repeat(64));
    const map = await dumpBlockMap("for user-action targeting");
    const plainAbovePlanning = [];
    for (let i = 0; i < map.length - 1; i++) {
      const cur = map[i].text;
      const nxt = map[i + 1].text;
      const curPlain = /TODO/.test(cur);
      const nxtIsInteresting =
        /child|consec|collapsed|referenced|deep planning|with deadline|last of day/i.test(nxt);
      if (curPlain && nxtIsInteresting) plainAbovePlanning.push(i);
    }
    log(`Plain blocks immediately above an interesting block: idx ${JSON.stringify(plainAbovePlanning)}`);
    for (const bi of plainAbovePlanning) {
      traceRows = [];
      log(`\n  # click idx ${bi} (${JSON.stringify(map[bi].text)}), ArrowDown x2`);
      await clickBlock(bi, "end");
      await probe(`click-${bi}`);
      await browser.keys(["ArrowDown"]);
      await sleep(450);
      await probe(`↓1`);
      await browser.keys(["ArrowDown"]);
      await sleep(450);
      await probe(`↓2`);
      const lost = traceRows.filter((r) => !r.isEditor && !r.tag.startsWith("click"));
      if (lost.length) {
        log(`    !! CARET LOST after click idx ${bi}:`);
        for (const r of lost)
          log(`       ${r.tag}: editingId=${r.editingId} editingIdx=${r.editingIdx} focusDeferred=${r.focusDeferred} ae=${r.aeTag}.${r.aeCls}`);
        results.push({ label: `user-action click-${bi}`, trace: [...traceRows], losses: [...lost] });
      }
    }
  }

  // ---- Analysis --------------------------------------------------------------
  log("\n" + "=".repeat(64));
  log("ANALYSIS");
  log("=".repeat(64));

  log("\n--- Column preservation (block-crossing landings) ---");
  for (const res of results) {
    const downCross = [];
    const upCross = [];
    let prev = -999;
    for (const r of res.trace) {
      if (!r.isEditor) { prev = -999; continue; }
      if (r.tag.startsWith("↓") && r.idx !== prev && prev !== -999)
        downCross.push(`idx${r.idx}:sel=${r.sel}`);
      if (r.tag.startsWith("↑") && r.idx !== prev && prev !== -999)
        upCross.push(`idx${r.idx}:sel=${r.sel}`);
      prev = r.idx;
    }
    if (downCross.length || upCross.length) {
      log(`  [${res.label}]`);
      if (downCross.length) log(`    Down landings: ${downCross.join("  ")}`);
      if (upCross.length) log(`    Up   landings: ${upCross.join("  ")}`);
    }
  }

  log("\n--- Caret losses (all sweeps) ---");
  const allLosses = results.flatMap((r) => r.losses.map((l) => ({ sweep: r.label, ...l })));
  if (allLosses.length === 0) log("  NONE across all sweeps.");
  else {
    for (const l of allLosses)
      log(`  sweep="${l.sweep}" ${l.tag}: editingId=${l.editingId} editingIdx=${l.editingIdx} focusDeferred=${l.focusDeferred} ae=${l.aeTag}.${l.aeCls}`);
  }
  } // end non-agenda scenario

  // ---- Write findings (append) -----------------------------------------------
  const notesDir = "/aux/koutecky/logseq/logseq-claude/subagent-tasks/notes";
  fs.mkdirSync(notesDir, { recursive: true });
  const outPath = path.join(notesDir, "caret-updown-repro-findings.md");
  const label = process.env.CARET_LABEL || "";
  const header =
    MODE === "agenda"
      ? `\n\n---\n\n# Round 3: agenda duplicate — ${label} (${new Date().toISOString()})\n`
      : MODE === "journal"
      ? `\n\n---\n\n# Round 2: journal feed + structure (${new Date().toISOString()})\n`
      : `\n\n---\n\n# Round 1b: bug-B goal-column re-confirm (${new Date().toISOString()})\n`;
  const section = [
    header,
    `Mode: ${MODE}. Blocks: ${blockCount}. date-chips: ${chipCount}.`,
    "",
    "```",
    ...lines,
    "```",
    "",
  ].join("\n");
  fs.appendFileSync(outPath, section);
  console.log(`\nAppended results to ${outPath}`);
} catch (e) {
  log(`\nE2E ERROR: ${String(e).split("\n").slice(0, 8).join(" | ")}`);
  process.exitCode = 1;
  const notesDir = "/aux/koutecky/logseq/logseq-claude/subagent-tasks/notes";
  fs.mkdirSync(notesDir, { recursive: true });
  fs.appendFileSync(
    path.join(notesDir, "caret-updown-repro-findings.md"),
    `\n\n# ${MODE} run ABORTED (${new Date().toISOString()})\n\n\`\`\`\n${lines.join("\n")}\n\`\`\`\n`
  );
} finally {
  try { await browser?.deleteSession(); } catch {}
  td.kill("SIGKILL");
  xvfb?.kill("SIGKILL");
}
