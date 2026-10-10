// Scenarios, fixtures and the metric registry for scripts/bench-storage-ab.mjs
// (STEP3-DESIGN §13). Each scenario runs in ONE app session on a private graph
// copy and records metrics through the Session ("<scenario>.<metric>" names).
// What can run against the base today is implemented against its real UI and IPC;
// parts that need candidate-only behaviour read the candidate's bench events
// (docs/bench-storage-ab.md) and are marked candidateOnly: they are UNVALIDATED
// until a candidate binary exists.
import fs from "node:fs";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { answerNativeDialog } from "./e2e-native-dialog.mjs";
import { makeToken, mib } from "./bench-ab-session.mjs";
import { Adapter, armWatch, readWatch } from "./bench-ab-adapters.mjs";
import { describe, keyToPublish, typedPrefixIn, allSeenAt, slopePerMinute, summarizeIo, parseNdjson, median } from "./bench-ab-stats.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const pad = (n, w = 3) => String(n).padStart(w, "0");
const MARKER_LEN = 3; // "qzx": keys before the marker is complete cannot be matched

// -- fixtures ---------------------------------------------------------------------
export function writeFixtures(dir) {
  const pages = path.join(dir, "pages");
  fs.mkdirSync(pages, { recursive: true });
  fs.mkdirSync(path.join(dir, "journals"), { recursive: true });
  const w = (name, text) => fs.writeFileSync(path.join(pages, `${name}.md`), text);
  w("Bench Save", "- save target\n");
  const sixty = [];
  for (let i = 0; i < 20; i++) {
    sixty.push(`- Section ${i}: the quick brown fox jumps over the lazy dog ${i}`);
    sixty.push(`  - Detail ${i}a with some [[Bench Hub]] context and enough words to look real`);
    sixty.push(`  - Detail ${i}b closing remark for section ${i}, nothing unusual`);
  }
  w("Bench Save 60", sixty.join("\n") + "\n");
  const big = [];
  for (let i = 0; i < 1500; i++) big.push(`- Big ${i}: pack my box with five dozen liquor jugs, then sphinx of black quartz judge my vow ${i}`);
  w("Bench Save Big", big.join("\n") + "\n");
  w("Bench Hub", "- hub block\n");
  for (let i = 0; i < 200; i++) w(`Bench Ref ${pad(i)}`, `- linked [[Bench Hub]]\n- unlinked Bench Hub mention ${i}\n`);
  w("Bench Del", "- doomed block one\n- doomed block two\n- doomed block three\n");
  w("Bench ExtHeld", "- held block one\n- held block two\n");
  for (let i = 0; i < 20; i++) w(`Bench Ext ${pad(i)}`, `- unheld block ${i}\n`);
  w("Bench RefSrc", "- source block\n");
  w("Bench RefTarget", "- refpick target alpha\n");
  for (let i = 0; i < 50; i++) {
    w(`Bench Draft ${pad(i)}`, `- draft target ${i}\n- second line ${i}\n`);
    w(`Bench DraftBig ${pad(i)}`, sixty.map((l) => l.replace("Section", `Draft${i} Section`)).join("\n") + "\n");
  }
}

function pageFile(s, name) { return path.join(s.graph, "pages", `${name}.md`); }
const sh = (cmd, args) => spawnSync(cmd, args, { encoding: "utf8" });

function chmodPages(s, mode) { fs.chmodSync(path.join(s.graph, "pages"), mode); }

/** Click the first block until an editor opens: the time of the first success is the
 *  "first editable" instant (a click on a not-yet-interactive page is ignored). */
async function waitEditable(s, timeoutMs = 60000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const open = await s.browser.execute(() => !!document.querySelector("textarea.block-editor"));
    if (open) return;
    try { await s.browser.$(".ls-block .block-content-wrapper").click(); } catch { /* not interactive yet */ }
    if (await s.browser.execute(() => !!document.querySelector("textarea.block-editor"))) return;
    if (Date.now() > deadline) throw new Error("no block became editable");
    await sleep(25);
  }
}

// -- shared measured edits --------------------------------------------------------
/** One isolated edit: type `n` characters, wait for the publish carrying them. */
async function isolatedEdit(s, prefix, page, code, n = 12) {
  await s.openPage(page);
  await s.settle();
  await s.journey(`${prefix}.iso`);
  await s.enterFirstBlock();
  const token = makeToken(code, n);
  const since = Date.now();
  await s.typeRange(token, 0, n, 100);
  const typed = s.keysSince(await s.records(), since);
  const full = token.slice(0, typed.length);
  const { hit, records } = await s.waitPublished(full, since);
  s.m(`${prefix}.afterLastKeyMs`, hit.t - typed.at(-1).t);
  s.m(`${prefix}.isoKeys`, typed.length);
  s.typingStats(records, typed, `${prefix}.iso`);
  await s.leaveEditor();
  await s.settle();
  return { token: full, since, hit };
}

/** A 5 s burst at 10 keys/s, then the tail until the final publish. */
async function burstEdit(s, prefix, page, code, { seconds = 5, intervalMs = 100 } = {}) {
  await s.openPage(page);
  await s.settle();
  await s.journey(`${prefix}.burst`);
  await s.enterFirstBlock();
  const n = Math.round((seconds * 1000) / intervalMs);
  const token = makeToken(code, n + 8);
  const since = Date.now();
  await s.typeRange(token, 0, n, intervalMs);
  const typed = s.keysSince(await s.records(), since);
  const full = token.slice(0, typed.length);
  const { hit, records } = await s.waitPublished(full, since);
  const keys = typed.map((k, idx) => ({ i: idx + 1, t: k.t })).filter((k) => k.i > MARKER_LEN);
  const pubs = records.publishes.filter((p) => p.t >= since).map((p) => ({ t: p.t, k: typedPrefixIn(p.text, full) }));
  const { latencies, lost } = keyToPublish(keys, pubs);
  const d = describe(latencies);
  s.m(`${prefix}.burstKeys`, typed.length);
  s.m(`${prefix}.burstPublishCount`, pubs.filter((p) => p.k > MARKER_LEN).length);
  s.m(`${prefix}.burstTailMs`, hit.t - typed.at(-1).t);
  s.m(`${prefix}.burstLostKeys`, lost.length);
  if (d) { s.m(`${prefix}.burstKeyToPublishP50Ms`, d.median); s.m(`${prefix}.burstKeyToPublishP95Ms`, d.p95); s.addSeries(`${prefix}.burstKeyToPublishMs`, latencies); }
  s.typingStats(records, typed, `${prefix}.burst`);
  s.longTasks(records, `${prefix}.burst`, `${prefix}.burst`);
  const admit = records.rtt.filter((r) => r.cmd === "page_submit" && r.t0 >= since).map((r) => r.ms);
  const da = describe(admit);
  if (da) { s.m(`${prefix}.admissionRttP95Ms`, da.p95); s.addSeries(`${prefix}.admissionRttMs`, admit); }
  const saveRtt = records.rtt.filter((r) => r.cmd === "save_pages" && r.t0 >= since).map((r) => r.ms);
  if (saveRtt.length) s.addSeries(`${prefix}.saveRttMs`, saveRtt);
  await s.leaveEditor();
  await s.settle();
  return { since, hit, typed };
}

// -- iotrace (unit cost) ------------------------------------------------------------
async function startTracer(s) {
  const bin = path.join(path.dirname(s.dir), "..", "bin-iotrace");
  fs.mkdirSync(path.dirname(bin), { recursive: true });
  if (!fs.existsSync(bin)) {
    const cc = spawnSync("gcc", ["-O2", "-Wall", "-o", bin, path.join(HERE, "iotrace.c")], { encoding: "utf8" });
    if (cc.status !== 0) throw new Error(`cannot build iotrace (needs gcc on x86_64 Linux): ${cc.stderr.slice(0, 300)}`);
  }
  const pid = s.pid();
  if (!pid) throw new Error("app pid not found for tracing");
  const file = path.join(s.dir, "iotrace.ndjson");
  const child = spawn(bin, [String(pid), file, s.graph, path.join(s.xdg)], { stdio: ["ignore", "pipe", "inherit"] });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("iotrace did not become ready")), 20000);
    child.stdout.on("data", (d) => { if (String(d).includes("ready")) { clearTimeout(timer); resolve(); } });
    child.once("exit", (code) => { clearTimeout(timer); reject(new Error(`iotrace exited early (${code}); is ptrace allowed?`)); });
  });
  await sleep(300);
  return { child, file };
}

async function stopTracer(tracer) {
  const exited = new Promise((resolve) => tracer.child.once("exit", resolve));
  tracer.child.kill("SIGTERM");
  await Promise.race([exited, sleep(10000)]);
  return parseNdjson(fs.readFileSync(tracer.file, "utf8"));
}

const UNIT_FIELDS = ["bytes", "graphBytes", "appDataBytes", "writeCalls", "filesCreated", "filesTouched", "renames", "fileSyncs", "dirSyncs", "syncCalls", "unlinks", "mkdirs"];

// -- scenarios --------------------------------------------------------------------
export const SCENARIOS = {
  launch: {
    title: "cold launch to first visible page and first editable page; RSS",
    timeoutMs: () => 150000, maxRuns: (o) => o.runs,
    async run(s) {
      s.m("launch.firstPageMs", await s.launch());
      await s.step("editable", async () => {
        await waitEditable(s);
        s.m("launch.firstEditableMs", performance.now() - s.launchPerf0);
      });
      const r = s.rss();
      s.m("launch.rssMainMiB", mib(r.main));
      s.m("launch.rssTreeMiB", mib(r.tree));
    },
  },

  typing: {
    title: "keystroke -> Published, 1-block and 60-block pages (plus a 1500-block page, where a save is long enough to type during), isolated and during a 5 s burst; typing latency and long tasks",
    timeoutMs: () => 240000, maxRuns: (o) => o.runs, candidateOnly: true,
    async run(s) {
      s.m("typing.firstPageMs", await s.launch());
      for (const [prefix, page, code] of [["typing1", "Bench Save", "a"], ["typing60", "Bench Save 60", "b"], ["typingBig", "Bench Save Big", "c"]]) {
        await s.step(`${prefix}.iso`, () => isolatedEdit(s, prefix, page, code));
        await s.step(`${prefix}.burst`, () => burstEdit(s, prefix, page, `${code}b`));
      }
    },
  },

  delete: {
    title: "delete a page: source gone and custody complete (trash entry)",
    timeoutMs: () => 150000, maxRuns: (o) => o.runs, candidateOnly: true,
    async run(s) {
      await s.launch();
      await s.openPage("Bench Del");
      await s.settle();
      await s.journey("delete");
      await s.browser.execute(() => document.querySelector("h1.page-title")
        .dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 200 })));
      await s.browser.$('[data-page-action-id="delete-page"]').waitForExist({ timeout: 10000 });
      await s.browser.execute(() => document.querySelector('[data-page-action-id="delete-page"]').click());
      const source = pageFile(s, "Bench Del");
      // The base trashes into <graph>/logseq/.tine-trash/pages (search_edit_tests.rs
      // delete_page_moves_to_trash_recoverable); the candidate's location is searched
      // under both roots so a relocation is a measured fact, not a false failure.
      const trashRoots = [path.join(s.graph, "logseq", ".tine-trash"), path.join(s.graph, ".tine-trash")];
      await answerNativeDialog("yes", { env: s.env });
      const answered = Date.now();
      const seen = {};
      const deadline = Date.now() + 60000;
      while (Date.now() < deadline && !(seen.gone && seen.trash && seen.toast)) {
        const now = Date.now();
        if (!seen.gone && !fs.existsSync(source)) seen.gone = now;
        if (!seen.trash && trashRoots.some((t) => fs.existsSync(t) && JSON.stringify(fs.readdirSync(t, { recursive: true })).includes("Bench Del"))) seen.trash = now;
        if (!seen.toast && await s.browser.execute(() => [...document.querySelectorAll(".toast")].some((e) => e.textContent.includes("Deleted")))) seen.toast = now;
        await sleep(5);
      }
      const r = await s.records();
      const call = r.probe.calls.find((c) => (c.cmd === "delete_page" || c.cmd === "page_delete") && c.t1 != null);
      const t0 = call ? call.t0 : answered;
      s.note("deleteCommand", call ? call.cmd : "not observed");
      if (call) s.m("delete.ipcMs", call.t1 - call.t0);
      if (seen.gone) s.m("delete.sourceGoneMs", seen.gone - t0);
      if (seen.trash) s.m("delete.trashEntryMs", seen.trash - t0);
      if (seen.toast) s.m("delete.toastMs", seen.toast - t0);
      if (seen.gone && seen.trash) s.m("delete.custodyCompleteMs", Math.max(seen.gone, seen.trash, call?.t1 ?? 0) - t0);
      const ev = r.extra.custodyComplete.find((e) => String(e.path ?? "").includes("Bench Del"));
      if (ev) s.m("delete.custodyEventMs", ev.t - t0);
      if (!seen.gone || !seen.trash) throw new Error(`delete did not complete: ${JSON.stringify(seen)}`);
    },
  },

  rename: {
    title: "rename a page with 200 referrers: referrers rewritten and visible in the index",
    timeoutMs: () => 180000, maxRuns: (o) => o.runs,
    async run(s) {
      await s.launch();
      await s.openPage("Bench Hub");
      await s.settle();
      await s.journey("rename");
      await s.browser.execute(() => document.querySelector("h1.page-title")
        ?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window })));
      await s.browser.$(".page-title-input").waitForExist({ timeout: 10000 });
      const t0 = Date.now();
      await s.browser.execute((name) => {
        const input = document.querySelector(".page-title-input");
        input.focus();
        input.value = name;
        input.dispatchEvent(new Event("input", { bubbles: true }));
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      }, "Bench Hub Renamed");
      const renamed = pageFile(s, "Bench Hub Renamed");
      const old = pageFile(s, "Bench Hub");
      const seen = {};
      const referrersDone = () => {
        for (let i = 0; i < 200; i++) {
          try { if (!fs.readFileSync(pageFile(s, `Bench Ref ${pad(i)}`), "utf8").includes("[[Bench Hub Renamed]]")) return false; } catch { return false; }
        }
        return true;
      };
      const deadline = Date.now() + 90000;
      while (Date.now() < deadline && !(seen.moved && seen.refs && seen.index && seen.ipc)) {
        const now = Date.now();
        if (!seen.moved && fs.existsSync(renamed) && !fs.existsSync(old)) seen.moved = now;
        if (!seen.refs && referrersDone()) seen.refs = now;
        if (!seen.index) {
          const count = await s.browser.execute(() => Number(document.querySelector(".linked-references .references-count")?.textContent) || 0);
          const title = await s.browser.execute(() => document.querySelector("h1.page-title")?.textContent?.trim());
          if (count >= 200 && title === "Bench Hub Renamed") seen.index = now;
        }
        if (!seen.ipc) {
          const r = await s.records();
          const call = r.probe.calls.find((c) => (c.cmd === "rename_page" || c.cmd === "page_rename") && c.t1 != null);
          if (call) { seen.ipc = call.t1; s.m("rename.ipcMs", call.t1 - call.t0); if (!call.ok) s.note("renameCallNotOk", true); }
        }
        if (await s.browser.$(".conflict-banner").isExisting()) throw new Error("rename raised a conflict");
        await sleep(20);
      }
      if (seen.moved) s.m("rename.fileMovedMs", seen.moved - t0);
      if (seen.refs) s.m("rename.referrersWrittenMs", seen.refs - t0);
      if (seen.index) s.m("rename.indexVisibleMs", seen.index - t0);
      const r = await s.records();
      s.longTasks(r, "rename", "rename");
      if (!(seen.moved && seen.refs && seen.index)) throw new Error(`rename incomplete: ${JSON.stringify(seen)}`);
    },
  },

  external: {
    title: "external-edit burst across a held and 20 unheld pages: time until visible; no conflict on clean pages; mail parse cost",
    timeoutMs: () => 150000, maxRuns: (o) => o.runs, candidateOnly: true,
    async run(s) {
      await s.launch();
      await s.openPage("Bench ExtHeld");
      await s.settle(1500);
      await s.journey("external");
      const heldFile = pageFile(s, "Bench ExtHeld");
      const heldText = "- held block one\n- held block extburstheld edited externally\n";
      const t0 = Date.now();
      for (let i = 0; i < 20; i++) fs.writeFileSync(pageFile(s, `Bench Ext ${pad(i)}`), `- unheld extburst${i}x see [[Bench ExtHeld]]\n`);
      fs.writeFileSync(heldFile, heldText);
      const seen = {};
      const deadline = Date.now() + 60000;
      while (Date.now() < deadline && !(seen.held && seen.unheld)) {
        const state = await s.browser.execute(() => ({
          held: document.body.innerText.includes("extburstheld"),
          refs: Number(document.querySelector(".linked-references .references-count")?.textContent) || 0,
          conflict: !!document.querySelector(".conflict-banner"),
        }));
        const now = Date.now();
        if (state.conflict) seen.conflict = true;
        if (!seen.held && state.held) seen.held = now;
        if (!seen.unheld && state.refs >= 20) seen.unheld = now;
        await sleep(10);
      }
      if (seen.held) s.m("ext.heldVisibleMs", seen.held - t0);
      if (seen.unheld) s.m("ext.unheldVisibleMs", seen.unheld - t0);
      await s.settle(1500);
      s.m("ext.conflicts", seen.conflict ? 1 : 0);
      // A clean held page must not be rewritten by Tine: its bytes are still the external ones.
      s.m("ext.cleanPageRewritten", fs.readFileSync(heldFile, "utf8") === heldText ? 0 : 1);
      const r = await s.records();
      s.longTasks(r, "external", "ext");
      const parse = r.extra.mailParse.map((e) => e.parse_us).filter(Number.isFinite);
      const dp = describe(parse);
      if (dp) { s.m("ext.mailParseCount", dp.n); s.m("ext.mailParseP50Us", dp.median); s.m("ext.mailParseP95Us", dp.p95); }
      if (!seen.held || !seen.unheld) throw new Error(`external edits not all visible: ${JSON.stringify(seen)}`);
    },
  },

  blockref: {
    title: "block-reference insertion: pick -> reference shown, and the target's id published",
    timeoutMs: () => 120000, maxRuns: (o) => o.runs,
    async run(s) {
      await s.launch();
      await s.openPage("Bench RefSrc");
      await s.settle();
      await s.journey("blockref");
      await s.enterFirstBlock();
      for (const key of ["(", "(", ..."refpick"]) await s.browser.keys([key]);
      await s.browser.waitUntil(() => s.browser.execute(() =>
        [...document.querySelectorAll(".autocomplete .ac-item")].some((e) => /refpick target/.test(e.textContent))),
      { timeout: 20000, interval: 50, timeoutMsg: "the block picker never offered the target" });
      await s.browser.execute(armWatch, "blockref", []);
      const t0 = Date.now();
      await s.browser.keys(["Enter"]);
      let shown = null;
      const deadline = Date.now() + 30000;
      while (Date.now() < deadline && shown == null) {
        const value = await s.browser.execute(() => document.querySelector("textarea.block-editor")?.value ?? "");
        if (/\(\([0-9a-f-]{36}\)\)/.test(value)) shown = Date.now();
        else await sleep(5);
      }
      if (shown == null) throw new Error("the picked reference never appeared in the editor");
      const watch = await s.browser.execute(readWatch);
      s.m("blockref.shownMs", watch?.dt ?? shown - t0); // the in-page stopwatch when it fired
      // The reference is meaningful only once the id is on disk: the first publish carrying `id::`.
      for (;;) {
        const r = await s.records();
        const hit = r.publishes.find((p) => p.t >= t0 && /id:: [0-9a-f-]{36}/.test(p.text));
        if (hit) { s.m("blockref.targetPublishedMs", hit.t - t0); break; }
        if (Date.now() - t0 > 30000) throw new Error("no publish carried the target's id::");
        await sleep(20);
      }
    },
  },

  custody: {
    title: "draft custody under a failing save and under a conflict (last key -> draft durable)",
    timeoutMs: () => 150000, maxRuns: (o) => o.runs, candidateOnly: true,
    async run(s) {
      await s.launch();
      await s.step("fail", async () => {
        chmodPages(s, 0o555);
        try {
          await s.openPage("Bench Draft 000");
          await s.settle();
          await s.journey("custody.fail");
          await s.enterFirstBlock();
          const token = makeToken("cf", 10);
          const since = Date.now();
          await s.typeRange(token, 0, 10, 100);
          const typed = s.keysSince(await s.records(), since);
          const { hit } = await s.waitDraft(token.slice(0, typed.length), since);
          s.m("custody.failMs", hit.t - typed.at(-1).t);
          await s.leaveEditor();
        } finally { chmodPages(s, 0o755); }
      });
      await s.step("conflict", async () => {
        await s.openPage("Bench Draft 001");
        await s.settle();
        await s.journey("custody.conflict");
        await s.enterFirstBlock();
        const token = makeToken("cc", 8);
        const since = Date.now();
        await s.typeRange(token, 0, 8, 100);
        fs.writeFileSync(pageFile(s, "Bench Draft 001"), "- externally replaced while the window was typing\n");
        const typed = s.keysSince(await s.records(), since);
        const { hit } = await s.waitDraft(token.slice(0, typed.length), since);
        s.m("custody.conflictMs", hit.t - typed.at(-1).t);
        await s.leaveEditor();
      });
    },
  },

  drafts: draftLaunch("launch with 0 and 20 recovered drafts", [0, 20], "Bench Draft", "drafts"),
  draftscale: draftLaunch("launch scaling over recovered-draft count (1, 10, 50) and bytes (small vs 60-block pages)",
    [[1, "S"], [10, "S"], [50, "S"], [1, "B"], [10, "B"], [50, "B"]], null, "draftsScale", 3),

  carry2: carryScenario(2),
  carry5: carryScenario(5),

  unitcost: {
    title: "unit cost per edit via a syscall trace: bytes, files, renames and sync calls (1-block, 60-block, at risk, burst, delete)",
    timeoutMs: () => 360000, maxRuns: (o) => Math.min(o.runs, 3),
    async run(s) {
      await s.launch();
      const tracer = await startTracer(s);
      const windows = [];
      const roots = { graph: s.graph, appData: s.xdg };
      const mark = (name, from, to, extra = {}) => windows.push({ name, from, to, ...extra });
      let fatal = null;
      try {
        // Startup activity first (the base writes a launch-time graph backup under app data for
        // ~10 s after the window opens): wait at least 15 s and until the trace file is quiet for 3 s, report that
        // burst as "startup", then take the idle noise floor on a quiet app.
        const startupFrom = Date.now();
        let lastSize = -1;
        let lastChange = Date.now();
        while ((Date.now() - lastChange < 3000 || Date.now() - startupFrom < 15000) && Date.now() - startupFrom < 90000) {
          await sleep(250);
          const size = fs.statSync(tracer.file).size;
          if (size !== lastSize) { lastSize = size; lastChange = Date.now(); }
        }
        mark("startup", startupFrom, lastChange);
        const idleFrom = Date.now();
        await sleep(10000);
        mark("idle10s", idleFrom, Date.now());

        for (const [name, page, code] of [["edit1", "Bench Save", "u"], ["edit60", "Bench Save 60", "v"]]) {
          await s.step(name, async () => {
            await s.openPage(page);
            await s.settle(1500);
            await s.enterFirstBlock();
            for (let k = 0; k < 5; k++) {
              const token = makeToken(`${code}${k}`, 8);
              const since = Date.now();
              await s.typeRange(token, 0, 8, 100);
              const typed = s.keysSince(await s.records(), since);
              const { hit, records } = await s.waitPublished(token.slice(0, typed.length), since);
              await sleep(1500);
              const ipc = records.rtt.filter((r) => r.t0 >= since && (r.cmd === "save_pages" || r.cmd === "page_submit")).length;
              mark(name, since - 100, Date.now(), { ipc, publishes: records.publishes.filter((p) => p.t >= since).length });
              await s.settle(500);
            }
            await s.leaveEditor();
          });
        }
        for (const [name, page, code] of [["risk1", "Bench Draft 002", "w"], ["risk60", "Bench DraftBig 002", "x"]]) {
          await s.step(name, async () => {
            chmodPages(s, 0o555);
            try {
              await s.openPage(page);
              await s.settle(1500);
              await s.enterFirstBlock();
              for (let k = 0; k < 5; k++) {
                const token = makeToken(`${code}${k}`, 8);
                const since = Date.now();
                await s.typeRange(token, 0, 8, 100);
                const typed = s.keysSince(await s.records(), since);
                await s.waitDraft(token.slice(0, typed.length), since);
                await sleep(1500);
                mark(name, since - 100, Date.now());
                await s.settle(500);
              }
              await s.leaveEditor();
            } finally { chmodPages(s, 0o755); }
          });
        }
        await s.step("burst1", async () => {
          const from = Date.now();
          await burstEdit(s, "unitburst", "Bench Save", "ub");
          mark("burst1", from, Date.now() + 1500);
          await sleep(1500);
        });
        await s.step("delete", async () => {
          await s.openPage("Bench Del");
          await s.settle(1500);
          await s.browser.execute(() => document.querySelector("h1.page-title")
            .dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 200 })));
          await s.browser.$('[data-page-action-id="delete-page"]').waitForExist({ timeout: 10000 });
          await s.browser.execute(() => document.querySelector('[data-page-action-id="delete-page"]').click());
          const from = Date.now();
          await answerNativeDialog("yes", { env: s.env });
          for (let i = 0; i < 100 && fs.existsSync(pageFile(s, "Bench Del")); i++) await sleep(50);
          await sleep(2000);
          mark("delete", from, Date.now());
        });
      } catch (error) { fatal = error; }
      const events = await stopTracer(tracer);
      s.note("iotraceEvents", events.length);
      const byName = new Map();
      for (const w of windows) {
        const io = summarizeIo(events, w.from, w.to, roots);
        io.ipcCalls = w.ipc ?? null;
        if (!byName.has(w.name)) byName.set(w.name, []);
        byName.get(w.name).push(io);
      }
      for (const [name, list] of byName) {
        const metricName = name === "idle10s" ? "unit.idle10s" : `unit.${name}`;
        for (const f of UNIT_FIELDS) {
          const v = list.map((io) => io[f]);
          s.m(`${metricName}.${f}`, median(v));
        }
        const ipcs = list.map((io) => io.ipcCalls).filter(Number.isFinite);
        if (ipcs.length) s.m(`${metricName}.ipcCalls`, median(ipcs));
        s.note(`${metricName}.all`, list);
      }
      if (fatal) throw fatal;
    },
  },

  session: {
    title: "RSS (main process and process tree) and event-vector size over a scripted session (--session-minutes, default 30)",
    timeoutMs: (o) => o.sessionMinutes * 60000 + 240000, maxRuns: (o) => Math.min(o.runs, Number(o.sessionRuns ?? 1)), candidateOnly: true,
    async run(s, o) {
      await s.launch();
      const points = [];
      const pages = ["Bench Save", "Bench Save 60", "Bench Hub", "Bench RefSrc", "Bench ExtHeld", "Bench Draft 010", "Bench Ref 001", "Bench Del"];
      const start = Date.now();
      const end = start + o.sessionMinutes * 60000;
      let cycle = 0;
      while (Date.now() < end) {
        const page = pages[cycle % pages.length];
        await s.step(`cycle${cycle}`, async () => {
          await s.openPage(page);
          if (page !== "Bench Del") {
            await s.enterFirstBlock();
            const token = makeToken(`s${cycle % 36}`, 6);
            const since = Date.now();
            await s.typeRange(token, 0, 6, 100);
            await s.leaveEditor();
            const typed = s.keysSince(await s.records(), since);
            await s.waitPublished(token.slice(0, typed.length), since, 20000);
          }
          for (let i = 0; i < 5; i++) fs.writeFileSync(pageFile(s, `Bench Ext ${pad(i)}`), `- session ${cycle} extburst${i}x\n`);
        });
        const rss = s.rss();
        points.push({ x: Date.now() - start, main: mib(rss.main), tree: mib(rss.tree) });
        cycle++;
        const next = start + cycle * 30000;
        if (next > Date.now() && next < end) await sleep(next - Date.now());
      }
      const warm = points.filter((p) => p.x >= Math.min(120000, o.sessionMinutes * 60000 * 0.2));
      const use = warm.length >= 3 ? warm : points;
      s.m("session.rssMainStartMiB", use[0]?.main);
      s.m("session.rssMainEndMiB", use.at(-1)?.main);
      s.m("session.rssMainMaxMiB", Math.max(...use.map((p) => p.main)));
      s.m("session.rssMainSlopeMiBPerMin", slopePerMinute(use.map((p) => ({ x: p.x, y: p.main }))));
      s.m("session.rssTreeMaxMiB", Math.max(...use.map((p) => p.tree ?? 0)));
      s.m("session.rssTreeSlopeMiBPerMin", slopePerMinute(use.filter((p) => p.tree != null).map((p) => ({ x: p.x, y: p.tree }))));
      s.note("rssPoints", points);
      const r = await s.records();
      const stats = r.extra.hostStats;
      if (stats.length) {
        s.m("session.eventVectorMaxLen", Math.max(...stats.map((e) => e.events_len ?? 0)));
        s.m("session.eventVectorMaxBytes", Math.max(...stats.map((e) => e.events_bytes ?? 0)));
        s.m("session.eventVectorEndLen", stats.at(-1).events_len);
      }
    },
  },
};

function carryScenario(k) {
  const offsets = k === 2 ? [1, 4] : [1, 2, 3, 5, 7];
  return {
    title: `carry-over of unfinished tasks from ${k} source days: start -> visible, published${k === 2 || k === 5 ? " (and unfrozen, candidate-only)" : ""}`,
    timeoutMs: () => 150000, maxRuns: (o) => o.runs, candidateOnly: true,
    async run(s) {
      const needles = [];
      const today = new Date();
      offsets.forEach((offset, j) => {
        const d = new Date(today.getFullYear(), today.getMonth(), today.getDate() - offset);
        const name = `${d.getFullYear()}_${pad(d.getMonth() + 1, 2)}_${pad(d.getDate(), 2)}`;
        needles.push(`carrytask${j + 1}`);
        fs.writeFileSync(path.join(s.graph, "journals", `${name}.md`), `- TODO carrytask${j + 1} unfinished\n`);
      });
      await s.launch();
      await s.settle(1500);
      await s.journey(`carry${k}`);
      await s.browser.$(".carry-btn-days").waitForExist({ timeout: 30000 });
      await s.browser.execute(armWatch, "carry", needles);
      const t0 = Date.now();
      await s.browser.execute(() => document.querySelector(".carry-btn-days").click());
      let visible = null;
      const deadline = Date.now() + 60000;
      while (Date.now() < deadline && visible == null) {
        const text = await s.browser.execute(() => document.querySelector(".page-section")?.innerText ?? "");
        if (needles.every((n) => text.includes(n))) visible = Date.now();
        else await sleep(10);
      }
      if (visible == null) throw new Error("the carried tasks never appeared in today's journal");
      const watch = await s.browser.execute(readWatch);
      s.m(`carry${k}.visibleMs`, watch?.dt ?? visible - t0); // the in-page stopwatch when it fired (no WebDriver latency)
      let publishedAt = null;
      while (Date.now() < deadline && publishedAt == null) {
        const r = await s.records();
        publishedAt = allSeenAt(r.publishes.filter((p) => p.t >= t0), needles);
        if (publishedAt == null) await sleep(20);
      }
      if (publishedAt == null) throw new Error("the carried tasks were never published");
      s.m(`carry${k}.publishedMs`, publishedAt - t0);
      const r = await s.records();
      const unfreeze = r.extra.unfreeze.find((e) => e.t >= t0);
      if (unfreeze) s.m(`carry${k}.unfreezeMs`, unfreeze.t - t0);
      s.longTasks(r, `carry${k}`, `carry${k}`);
    },
  };
}

/** Launch with N recovered drafts, created by the real mechanism: saves fail (the pages
 *  directory is read-only), so each typed page is drafted; then a hard kill (a crash),
 *  permissions restored, and the relaunch is what is measured. */
function draftLaunch(title, configs, pagePrefix, tag, maxRunsCap = Infinity) {
  return {
    title, candidateOnly: true,
    timeoutMs: (o) => 150000 + 9000 * Math.max(...configs.map((c) => (Array.isArray(c) ? c[0] : c))) * (Array.isArray(configs[0]) ? configs.length : configs.length),
    maxRuns: (o) => Math.min(o.runs, maxRunsCap),
    async run(s) {
      for (const config of configs) {
        const [count, shape] = Array.isArray(config) ? config : [config, "S"];
        const label = `${tag}${shape === "B" ? "Big" : ""}${count}`;
        const prefix = shape === "B" ? "Bench DraftBig" : (pagePrefix ?? "Bench Draft");
        await s.step(label, async () => {
          // each configuration starts from the pristine corpus copy: re-prepare
          s.kill();
          s.prepare();
          const tokens = [];
          if (count > 0) {
            await s.launch({ tag: `seed-${label}` });
            chmodPages(s, 0o555);
            try {
              for (let i = 0; i < count; i++) {
                await s.openPage(`${prefix} ${pad(i)}`);
                await s.settle(300, 8000);
                await s.enterFirstBlock();
                const token = makeToken(`d${i}`, 10);
                const since = Date.now();
                await s.typeRange(token, 0, 10, 30);
                const typed = s.keysSince(await s.records(), since);
                const full = token.slice(0, typed.length);
                await s.waitDraft(full, since);
                tokens.push(full);
                await s.leaveEditor();
              }
            } finally {
              s.kill(); // a crash with every draft durable
              chmodPages(s, 0o755);
            }
            await sleep(500);
          }
          s.prepare({ keepGraph: true });
          const firstPage = await s.launch({ tag: `measure-${label}` });
          s.m(`${label}.firstPageMs`, firstPage);
          await waitEditable(s);
          s.m(`${label}.firstEditableMs`, performance.now() - s.launchPerf0);
          const rss = s.rss();
          s.m(`${label}.rssMainMiB`, mib(rss.main));
          const r = await s.records();
          const recovered = r.extra.launchRecovered.at(-1);
          if (recovered) s.m(`${label}.recoveredDrafts`, recovered.drafts);
          if (tokens.length) {
            // What the user sees. The base (draftStore.ts offerEarlier) raises a sticky toast
            // "Tine kept unsaved drafts of ..." whose Review action opens the recovery panel
            // (one .unsaved-recovery-entry per kept draft). A candidate that restores into the
            // page instead is covered by the page-text fallback below.
            const offered = await s.browser.waitUntil(() => s.browser.execute(() =>
              [...document.querySelectorAll(".toast")].some((e) => /kept unsaved drafts/.test(e.textContent))),
            { timeout: 15000, interval: 25 }).catch(() => false);
            if (offered) s.m(`${label}.offeredMs`, performance.now() - s.launchPerf0);
            let found = false;
            if (offered) {
              await s.browser.execute(() => [...document.querySelectorAll(".toast")]
                .find((e) => /kept unsaved drafts/.test(e.textContent))?.querySelector(".toast-action")?.click());
              await s.browser.$(".unsaved-recovery-panel").waitForExist({ timeout: 10000 }).catch(() => {});
              const panel = await s.browser.execute(() => ({
                entries: document.querySelectorAll(".unsaved-recovery-entry").length,
                text: document.querySelector(".unsaved-recovery-panel")?.innerText ?? "",
              }));
              s.m(`${label}.offeredEntries`, panel.entries);
              found = panel.text.includes(tokens[0]);
              await s.browser.execute(() => document.querySelector(".unsaved-recovery-panel button:nth-of-type(2)")?.click());
            }
            if (!found) {
              await s.openPage(`${prefix} ${pad(0)}`);
              found = (await s.browser.execute(() => document.body.innerText)).includes(tokens[0]);
            }
            s.m(`${label}.firstDraftRecovered`, found ? 1 : 0);
          }
          s.kill();
        });
      }
    },
  };
}

export const DEFAULT_SCENARIOS = ["launch", "typing", "delete", "rename", "external", "blockref", "carry2", "carry5", "custody", "drafts", "unitcost"];

// -- metric registry ----------------------------------------------------------------
/** WebKit rounds timers to 1 ms and the typing probe sees 6..10 ms, so a 1-3 ms move is quantization, not signal. */
const TIMER_FLOOR_MS = 3;
const METRIC_LIST = [
  ["launch.firstPageMs", "ms", "cold launch (request) to first painted page"],
  ["launch.firstEditableMs", "ms", "cold launch (request) to first block that opens an editor"],
  ["launch.rssMainMiB", "MiB", "RSS of the main process after first editable"],
  ["launch.rssTreeMiB", "MiB", "RSS of the app's process tree (webview helpers included)"],
  ...["typing1", "typing60", "typingBig"].flatMap((p) => [
    [`${p}.afterLastKeyMs`, "ms", "last key of an isolated edit -> Published (matched publish for that typed text)"],
    [`${p}.iso.typingP50Ms`, "ms", "typing latency (input -> next paint), isolated edit; 1 ms timer resolution", { absFloor: TIMER_FLOOR_MS }],
    [`${p}.iso.typingP95Ms`, "ms", "typing latency p95, isolated edit", { absFloor: TIMER_FLOOR_MS }],
    [`${p}.burstKeyToPublishP50Ms`, "ms", "continuous 5 s burst at 10 keys/s: keystroke -> first publish carrying it, p50"],
    [`${p}.burstKeyToPublishP95Ms`, "ms", "same, p95"],
    [`${p}.burstTailMs`, "ms", "burst: last key -> final Published"],
    [`${p}.burstPublishCount`, "count", "burst: publishes during/after the burst (write cost: SPEC-s2 §4.9 cap 3 s -> 1 s)", { neutral: true }],
    [`${p}.burstLostKeys`, "count", "burst: keys never covered by any publish (must be 0)", { neutral: true }],
    [`${p}.burstKeys`, "count", "burst: keystrokes delivered", { neutral: true }],
    [`${p}.burst.typingP50Ms`, "ms", "typing latency p50 during the burst", { absFloor: TIMER_FLOOR_MS }],
    [`${p}.burst.typingP95Ms`, "ms", "typing latency p95 during the burst", { absFloor: TIMER_FLOOR_MS }],
    [`${p}.burst.typingDuringSaveP50Ms`, "ms", "typing latency p50 for keys typed while a save was running", { absFloor: TIMER_FLOOR_MS }],
    [`${p}.burst.typingDuringSaveP95Ms`, "ms", "typing latency p95 for keys typed while a save was running", { absFloor: TIMER_FLOOR_MS }],
    [`${p}.burst.typingDuringSaveN`, "count", "keys typed while a save was running (few: saves take tens of ms)", { neutral: true }],
    [`${p}.burst.typingNearSaveP50Ms`, "ms", "typing latency p50 for keys typed during a save or up to 500 ms after it ended", { absFloor: TIMER_FLOOR_MS }],
    [`${p}.burst.typingNearSaveP95Ms`, "ms", "same, p95", { absFloor: TIMER_FLOOR_MS }],
    [`${p}.burst.typingNearSaveN`, "count", "keys in that window", { neutral: true }],
    [`${p}.burst.longTaskCount`, "count", "main-thread gaps over 100 ms during the burst"],
    [`${p}.burst.longTaskMaxMs`, "ms", "longest gap during the burst", { absFloor: 20 }],
    [`${p}.admissionRttP95Ms`, "ms", "candidate: page_submit round trip p95 during ordinary saves (R9)", { candidateOnly: true }],
  ]),
  ["delete.ipcMs", "ms", "delete command round trip"],
  ["delete.sourceGoneMs", "ms", "delete command start -> source file gone"],
  ["delete.trashEntryMs", "ms", "delete command start -> entry present in .tine-trash"],
  ["delete.toastMs", "ms", "delete command start -> 'Deleted' toast"],
  ["delete.custodyCompleteMs", "ms", "source gone and trash entry present"],
  ["delete.custodyEventMs", "ms", "candidate: custody_complete bench event", { candidateOnly: true }],
  ["rename.ipcMs", "ms", "rename command round trip"],
  ["rename.fileMovedMs", "ms", "submit -> renamed file present, old gone"],
  ["rename.referrersWrittenMs", "ms", "submit -> all 200 referrers rewritten on disk"],
  ["rename.indexVisibleMs", "ms", "submit -> the renamed page shows its 200 linked references"],
  ["ext.heldVisibleMs", "ms", "external edit burst -> held page shows the new text"],
  ["ext.unheldVisibleMs", "ms", "external edit burst -> 20 unheld pages visible in the index (linked references)"],
  ["ext.conflicts", "count", "conflict banners on clean pages (must be 0)", { neutral: true }],
  ["ext.cleanPageRewritten", "count", "clean held page rewritten by Tine (must be 0)", { neutral: true }],
  ["ext.mailParseP50Us", "us", "candidate: page-mail DTO parse cost p50 (mail_parse events)", { candidateOnly: true }],
  ["ext.mailParseP95Us", "us", "candidate: same, p95", { candidateOnly: true }],
  ["blockref.shownMs", "ms", "picker Enter -> ((uuid)) shown in the editor (in-page stopwatch)", { absFloor: 10 }],
  ["blockref.targetPublishedMs", "ms", "picker Enter -> publish carrying the target's id::"],
  ["custody.failMs", "ms", "last key -> draft durable, saves failing (pages dir read-only)"],
  ["custody.conflictMs", "ms", "last key -> draft durable, external edit under a dirty page"],
  ["carry2.visibleMs", "ms", "carry-over from 2 sources: click -> tasks shown (in-page stopwatch)", { absFloor: 15 }],
  ["carry2.publishedMs", "ms", "carry-over from 2 sources: click -> tasks published"],
  ["carry2.unfreezeMs", "ms", "candidate: click -> unfreeze event", { candidateOnly: true }],
  ["carry5.visibleMs", "ms", "carry-over from 5 sources: click -> tasks shown (in-page stopwatch)", { absFloor: 15 }],
  ["carry5.publishedMs", "ms", "carry-over from 5 sources: click -> tasks published"],
  ["carry5.unfreezeMs", "ms", "candidate: click -> unfreeze event", { candidateOnly: true }],
  ...["drafts0", "drafts20"].flatMap((p) => [
    [`${p}.firstPageMs`, "ms", "relaunch with recovered drafts -> first painted page"],
    [`${p}.firstEditableMs`, "ms", "relaunch with recovered drafts -> first editable"],
    [`${p}.offeredMs`, "ms", "launch -> the kept-drafts offer is visible (base: sticky toast)"],
    [`${p}.offeredEntries`, "count", "drafts listed by the recovery view (should equal the seeded count)", { neutral: true }],
    [`${p}.firstDraftRecovered`, "count", "the first drafted page came back with its typed text (1 = yes)", { neutral: true }],
  ]),
  ...["draftsScale1", "draftsScale10", "draftsScale50", "draftsScaleBig1", "draftsScaleBig10", "draftsScaleBig50"].flatMap((p) => [
    [`${p}.firstPageMs`, "ms", "scaling probe: relaunch with recovered drafts -> first page"],
    [`${p}.firstEditableMs`, "ms", "scaling probe: -> first editable"],
  ]),
  ["session.rssMainSlopeMiBPerMin", "MiB/min", "RSS growth of the main process over the scripted session (after warm-up); A/A on 5-minute sessions differed by ~1 MiB/min, so 1.0 is the provisional tolerance until a 30-minute A/A is run", { absFloor: 1.0 }],
  ["session.rssTreeSlopeMiBPerMin", "MiB/min", "RSS growth of the process tree"],
  ["session.rssMainMaxMiB", "MiB", "peak RSS of the main process"],
  ["session.eventVectorMaxLen", "count", "candidate: largest Host::events length sampled (host_stats)", { candidateOnly: true }],
  ["session.eventVectorMaxBytes", "B", "candidate: largest event-vector size sampled", { candidateOnly: true }],
];
for (const variant of ["startup", "idle10s", "edit1", "edit60", "risk1", "risk60", "burst1", "delete"]) {
  for (const f of UNIT_FIELDS) METRIC_LIST.push([`unit.${variant}.${f}`, f.endsWith("ytes") ? "B" : "count", `syscall trace, ${variant}: ${f} (median over its windows)`, { neutral: f !== "bytes" && f !== "syncCalls" }]);
}
METRIC_LIST.push(["unit.edit1.ipcCalls", "count", "IPC save/submit calls per edit, 1-block", { neutral: true }], ["unit.edit60.ipcCalls", "count", "IPC save/submit calls per edit, 60-block", { neutral: true }]);

export const METRICS = Object.fromEntries(METRIC_LIST.map(([id, unit, desc, flags = {}]) => [id, { unit, desc, ...flags }]));

export function metricInfo(name) {
  if (METRICS[name]) return METRICS[name];
  const unit = /MiB/.test(name) ? "MiB" : /Us$/.test(name) ? "us" : /Count$|N$|Keys$|Conflicts$/.test(name) ? "count" : /Ms$/.test(name) ? "ms" : "";
  return { unit, desc: "", neutral: unit === "count", candidateOnly: /admission|unfreeze|mailParse|eventVector/.test(name) };
}

export { Adapter };
