// e2e-caret.mjs — headless repro for Tine ArrowUp/Down caret bugs.
//
// Bug A: caret vanishes on SOME SCHEDULED/DEADLINE blocks during keyboard nav.
// Bug B: column preservation across blocks (goal-column fix).
//
// Two modes (env CARET_MODE):
//   page    (default) — single CaretTest page, Round-1 fixture (bug B confirm).
//   journal           — multi-day journal FEED with structural variants (bug A).
//
// Usage:
//   Xvfb :98 -screen 0 1400x1000x24 &
//   DISPLAY=:98 CARET_MODE=journal node scripts/e2e-caret.mjs
//
// Appends findings to subagent-tasks/notes/caret-updown-repro-findings.md.
// Does NOT modify Tine src/.

import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";

const MODE = process.env.CARET_MODE || "page";
const G = "/tmp/txdg-caret-g";
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");

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

// Round 3: agenda-duplicate scenario. TODAY = 2026-07-01. Several sibling blocks,
// with SCHEDULED/DEADLINE dates inside the agenda window (2026-06-24..2026-07-08)
// so the "Scheduled & Deadline" agenda lists them → each renders TWICE in the DOM.
const J_AGENDA = [
  "- TODO alpha one two three",
  "- TODO beta four five",
  "- TODO gamma six seven",
  "  SCHEDULED: <2026-07-01 Tue>",
  "- TODO delta eight",
  "  DEADLINE: <2026-07-03 Thu>",
  "- TODO epsilon nine ten",
  "- TODO zeta scheduled soon",
  "  SCHEDULED: <2026-06-30 Mon>",
  "- TODO eta deadline soon",
  "  DEADLINE: <2026-07-05 Sun>",
  "",
].join("\n");

function seed() {
  fs.rmSync(G, { recursive: true, force: true });
  fs.mkdirSync(`${G}/pages`, { recursive: true });
  fs.mkdirSync(`${G}/journals`, { recursive: true });
  fs.mkdirSync(`${G}/logseq`, { recursive: true });
  if (MODE === "agenda") {
    fs.writeFileSync(`${G}/journals/2026_07_01.md`, J_AGENDA);
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
console.log("DISPLAY=", process.env.DISPLAY, "MODE=", MODE);

const tdLog = fs.openSync("/tmp/td-caret.log", "w");
const td = spawn(
  TD,
  ["--port", "4444", "--native-port", "4445", "--native-driver", "/usr/bin/WebKitWebDriver"],
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
    if (wrapper) wrapper.click();
    else block.click();
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
  const info = await browser.execute(() => {
    const agenda = document.querySelector(".agenda-block");
    const copies = document.querySelectorAll(
      ".agenda-block .ls-block, .query-block .ls-block"
    ).length;
    const count = (t) =>
      [...document.querySelectorAll(".ls-block")].filter((b) =>
        (b.textContent || "").includes(t)
      ).length;
    return {
      hasAgenda: !!agenda,
      copies,
      gamma: count("gamma six seven"),
      delta: count("delta eight"),
      zeta: count("zeta scheduled soon"),
    };
  });
  log(`Agenda present: ${info.hasAgenda}   agenda .ls-block copies: ${info.copies}`);
  log(
    `DOM occurrences — gamma:${info.gamma} delta:${info.delta} zeta:${info.zeta}  (2 => duplicated in agenda)`
  );

  // Click a MAIN-outline block (NOT inside the agenda/query) by text substring.
  const clickMain = async (textSub, caret) => {
    await browser.execute((t) => {
      const blocks = [...document.querySelectorAll(".ls-block")];
      const main = blocks.find(
        (b) => !b.closest(".agenda-block, .query-block") && (b.textContent || "").includes(t)
      );
      if (!main) return;
      const w = main.querySelector(":scope > .block-main .block-content-wrapper");
      (w || main).click();
    }, textSub);
    await sleep(450);
    await browser.execute((c) => {
      const ae = document.activeElement;
      if (!(ae instanceof HTMLTextAreaElement)) return;
      const pos = c === "end" ? ae.value.length : Number(c);
      ae.setSelectionRange(pos, pos);
    }, caret === "end" ? "end" : String(caret));
    await sleep(150);
  };

  const aprobe = async (tag) => {
    const s = await browser.execute(() => {
      const ae = document.activeElement;
      const isEd = ae instanceof HTMLTextAreaElement && ae.classList.contains("block-editor");
      const inAgenda = ae && ae.closest ? !!ae.closest(".agenda-block, .query-block") : false;
      const em = document.querySelector(".block-main.editing");
      const eb = em ? em.closest(".ls-block") : null;
      const editingId = eb ? eb.getAttribute("data-block-id") : null;
      const editingInAgenda = eb ? !!eb.closest(".agenda-block, .query-block") : null;
      const sc = document.querySelector(".main-content");
      return {
        isEd,
        inAgenda,
        sel: isEd ? ae.selectionStart : null,
        val: isEd ? ae.value.slice(0, 28) : null,
        editingId: editingId ? editingId.slice(0, 8) : null,
        editingInAgenda,
        scrollTop: sc ? Math.round(sc.scrollTop) : -1,
        aeTag: ae ? ae.tagName : "null",
        aeCls: ae ? String(ae.className).slice(0, 34) : "",
      };
    });
    const verdict = !s.isEd ? "CARET_LOST" : s.inAgenda ? "STOLEN-BY-AGENDA" : "OK-main";
    log(
      `    ${tag.padEnd(16)} ${verdict.padEnd(16)} sel=${s.sel} inAgenda=${s.inAgenda}` +
        ` editInAgenda=${s.editingInAgenda} scrollTop=${s.scrollTop} eId=${s.editingId}` +
        ` ae=${s.aeTag}.${s.aeCls} val=${JSON.stringify(s.val)}`
    );
    return { tag, ...s };
  };

  const cases = [
    { name: "Down INTO gamma (from beta above)", start: "beta four five", key: "ArrowDown", caret: "end" },
    { name: "Up INTO gamma (from delta below)", start: "delta eight", key: "ArrowUp", caret: 0 },
    { name: "Down INTO delta (from gamma above)", start: "gamma six seven", key: "ArrowDown", caret: "end" },
    { name: "Up INTO delta (from epsilon below)", start: "epsilon nine ten", key: "ArrowUp", caret: 0 },
  ];
  const outcomes = [];
  for (const c of cases) {
    log(`\n  CASE: ${c.name}`);
    await clickMain(c.start, c.caret);
    const before = await aprobe("click(start)");
    await browser.keys([c.key]);
    await sleep(550);
    const after = await aprobe(c.key);
    const jump = Math.abs((after.scrollTop ?? 0) - (before.scrollTop ?? 0));
    const outcome = !after.isEd
      ? "CARET_LOST"
      : after.inAgenda
      ? "STOLEN-BY-AGENDA"
      : "OK-main";
    log(
      `    scrollTop ${before.scrollTop} -> ${after.scrollTop} (Δ=${jump}${jump > 50 ? "  <== VIEWPORT JUMP" : ""})  => ${outcome}`
    );
    outcomes.push({ case: c.name, outcome, jump, editingInAgenda: after.editingInAgenda });
  }
  log(`\n  --- AGENDA SCENARIO SUMMARY (${process.env.CARET_LABEL || "?"}) ---`);
  for (const o of outcomes)
    log(`    ${o.outcome.padEnd(16)} jump=${o.jump} editInAgenda=${o.editingInAgenda}  ${o.case}`);
  const anyBug = outcomes.some(
    (o) => o.outcome !== "OK-main" || o.editingInAgenda || o.jump > 50
  );
  log(`  VERDICT: ${anyBug ? "BUG PRESENT (steal/loss/jump seen)" : "CLEAN (all main-outline, no jump)"}`);
  return { hasAgenda: info.hasAgenda, copies: info.copies, outcomes, anyBug };
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
   await runAgendaScenario();
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
  const notesDir = "/aux/koutecky/logseq/logseq-claude/subagent-tasks/notes";
  fs.mkdirSync(notesDir, { recursive: true });
  fs.appendFileSync(
    path.join(notesDir, "caret-updown-repro-findings.md"),
    `\n\n# ${MODE} run ABORTED (${new Date().toISOString()})\n\n\`\`\`\n${lines.join("\n")}\n\`\`\`\n`
  );
} finally {
  try { await browser?.deleteSession(); } catch {}
  td.kill("SIGKILL");
}
