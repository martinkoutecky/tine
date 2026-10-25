// Stage 1e: real XTest -> IBus Hangul -> WebKit, fresh graph per journey.
// Run through spike-cm6-ime-env.sh with SPIKE_IME_SCRIPT pointing here.
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const out = process.env.SPIKE_IME_OUT;
if (!out || !process.env.IBUS_ADDRESS) throw new Error("Use the private X/DBus IME environment");
const scenarios = [
  ["commit", "commit ", 7],
  ["escape-cancel", "cancel ", 7],
  ["enter", "enter ", 6],
  ["backspace", "backspace ", 10],
  ["page-autocomplete", "[[", 2],
  ["page-autocomplete-cancel", "[[", 2],
  ["block-start", " tail", 0],
  ["wrapped-line", "wrapped words ".repeat(130), 1050],
].filter(([name]) => !process.env.SPIKE_IME_SCENARIOS || process.env.SPIKE_IME_SCENARIOS.split(",").includes(name));
const app = path.resolve(process.env.TINE_APP || "target/release/tine");
const report = {
  commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  app, binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  forcePreedit: process.env.SPIKE_FORCE_PREEDIT, engine: "IBus Hangul 1.5.4, two-set keyboard",
  cases: [], failureCapsules: [],
};
let runIndex = 0;
for (const mode of (process.env.SPIKE_IME_MODES || "textarea,cm6").split(",")) {
  for (const [scenario, base, at] of scenarios) {
    const dir = path.join(out, mode, scenario);
    const graph = path.join(dir, "graph");
    for (const sub of ["journals", "pages", "logseq"]) fs.mkdirSync(path.join(graph, sub), { recursive: true });
    for (const sub of ["data", "config", "cache"]) fs.mkdirSync(path.join(dir, sub), { recursive: true });
    const now = new Date();
    const stamp = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
    const page = path.join(graph, "journals", `${stamp}.md`);
    const disk = text => `- ${text.replace(/[ \t]+$/, "")}\n- Neighbour stays intact\n`;
    // Only fixture creation writes directly. All subsequent writes are Tine saves.
    fs.writeFileSync(page, `- ${base}\n- Neighbour stays intact\n`);
    fs.writeFileSync(path.join(graph, "pages", "한글.md"), "- Candidate page\n");
    fs.writeFileSync(path.join(graph, "logseq", "config.edn"), "{}\n");
    const log = fs.openSync(path.join(dir, "driver.log"), "w");
    const port = Number(process.env.SPIKE_IME_PORT || 4960) + runIndex++ * 2;
    const driver = spawn(process.env.TAURI_DRIVER || `${process.env.CARGO_HOME}/bin/tauri-driver`, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], {
      detached: true, stdio: ["ignore", log, log], env: { ...process.env, TINE_GRAPH: graph,
        XDG_DATA_HOME: path.join(dir, "data"), XDG_CONFIG_HOME: path.join(dir, "config"), XDG_CACHE_HOME: path.join(dir, "cache"),
        WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" },
    });
    const journey = { mode, scenario, base, at, checks: [], keys: [], snapshots: {} };
    report.cases.push(journey);
    const check = (name, pass, detail, policy = false) => {
      journey.checks.push({ name, status: policy ? "engine-policy" : pass ? "pass" : "fail", detail });
      console.log(JSON.stringify({ mode, scenario, ...journey.checks.at(-1) }));
    };
    let browser;
    try {
      await sleep(1200);
      browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
        connectionRetryTimeout: 30000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
      await ensureMainWindow(browser, { what: "real IME scenarios" });
      await browser.$(".page-title").waitForExist({ timeout: 60000 });
      await browser.$(".main-content .ls-block .block-content-wrapper").waitForExist({ timeout: 60000 });
      await browser.execute(mode => { sessionStorage.setItem("tine-spike-cm6", mode === "cm6" ? "1" : "0"); location.reload(); }, mode);
      await browser.$(".page-title").waitForExist({ timeout: 60000 });
      await browser.$(".main-content .ls-block .block-content-wrapper").waitForExist({ timeout: 60000 });
      await sleep(500);
      journey.runtime = await browser.execute(() => ({ userAgent: navigator.userAgent, platform: navigator.platform,
        choice: sessionStorage.getItem("tine-spike-cm6"), url: location.href }));
      // Install BEFORE CM6 mounts: its window capture handler stops composing
      // keys immediately, so a later document listener misses that causal trace.
      await browser.execute(() => {
        window.__imeEvents = [];
        window.__imeRead = () => {
          const host = document.querySelector(".cm6-spike");
          const v = host?.cm6View;
          const e = document.querySelector("textarea.block-editor");
          const text = v ? v.state.doc.toString() : e?.value ?? null;
          const anchor = v ? v.state.selection.main.anchor : e?.selectionStart ?? null;
          const head = v ? v.state.selection.main.head : e?.selectionEnd ?? null;
          return { text, anchor, head, dom: v?.contentDOM.innerHTML ?? null, focus: document.activeElement?.matches("textarea.block-editor, .cm-content") ?? false,
            blockId: (host || e)?.closest(".ls-block")?.dataset.blockId, composing: v?.compositionStarted ?? null };
        };
        for (const type of ["compositionstart", "compositionupdate", "compositionend", "beforeinput", "input", "keydown", "keyup"]) {
          window.addEventListener(type, e => {
            const item = { n: window.__imeEvents.length, time: performance.now(), type, data: e.data, key: e.key,
              code: e.code, keyCode: e.keyCode, shift: e.shiftKey, isComposing: e.isComposing,
              trusted: e.isTrusted, inputType: e.inputType, before: window.__imeRead() };
            window.__imeEvents.push(item);
            queueMicrotask(() => { item.defaultPrevented = e.defaultPrevented; item.after = window.__imeRead(); });
          }, true);
        }
      });
      await browser.execute(() => document.querySelector(".main-content .ls-block .block-content-wrapper").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
      await browser.$(mode === "cm6" ? ".cm-content" : "textarea.block-editor").waitForExist({ timeout: 10000 });
      execFileSync("ibus", ["engine", "xkb:us::eng"]);
      await browser.execute(at => {
        const v = document.querySelector(".cm6-spike")?.cm6View;
        if (v) { v.dispatch({ selection: { anchor: at }, scrollIntoView: true }); v.focus(); }
        else { const e = document.querySelector("textarea.block-editor"); e.setSelectionRange(at, at); e.focus(); }
      }, at);
      const ids = execFileSync("xdotool", ["search", "--onlyvisible", "--class", "tine"], { encoding: "utf8" }).trim().split("\n");
      execFileSync("xdotool", ["windowfocus", "--sync", ids.at(-1)]);
      const key = async (...keys) => {
        journey.keys.push({ keys, at: new Date().toISOString() });
        execFileSync("xdotool", ["key", "--clearmodifiers", "--delay", "100", ...keys]);
        await sleep(300);
      };
      const state = () => browser.execute(() => {
        const s = window.__imeRead();
        const v = document.querySelector(".cm6-spike")?.cm6View;
        const e = document.querySelector("textarea.block-editor");
        const rect = r => r && ({ left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height });
        let caret = null, first = null, last = null;
        if (v) {
          caret = rect(v.coordsAtPos(s.head)); first = rect(v.coordsAtPos(0)); last = rect(v.coordsAtPos(v.state.doc.length));
        } else if (e) {
          // Same-style offscreen mirror, read only; never alter the editor DOM.
          const mirror = document.createElement("div"), cs = getComputedStyle(e);
          for (const p of ["box-sizing", "width", "padding", "border", "font", "letter-spacing", "line-height", "white-space", "word-break", "overflow-wrap", "tab-size", "text-indent"]) mirror.style.setProperty(p, cs.getPropertyValue(p));
          mirror.style.position = "absolute"; mirror.style.top = "0"; mirror.style.left = "-9999px"; mirror.style.visibility = "hidden";
          const text = document.createTextNode(e.value + "\u200b"); mirror.append(text); document.body.append(mirror);
          const origin = mirror.getBoundingClientRect(), box = e.getBoundingClientRect(), range = document.createRange();
          const measure = at => {
            range.setStart(text, at); range.collapse(true);
            const r = range.getClientRects()[0];
            return r && { left: box.left + r.left - origin.left - e.scrollLeft, right: box.left + r.right - origin.left - e.scrollLeft,
              top: box.top + r.top - origin.top - e.scrollTop, bottom: box.top + r.bottom - origin.top - e.scrollTop, width: r.width, height: r.height };
          };
          caret = measure(s.head); first = measure(0); last = measure(e.value.length); mirror.remove();
        }
        const scrollers = [];
        for (let el = v?.contentDOM || e; el; el = el.parentElement) {
          if (el.scrollHeight > el.clientHeight || el.scrollTop) scrollers.push({ tag: el.tagName, class: el.className, top: el.scrollTop, left: el.scrollLeft,
            height: el.clientHeight, scrollHeight: el.scrollHeight, rect: rect(el.getBoundingClientRect()), overflow: getComputedStyle(el).overflowY });
        }
        const popup = document.querySelector(".autocomplete");
        const candidates = Array.from(document.querySelectorAll(v ? ".cm6-completions button" : ".autocomplete .ac-label"), el => el.textContent);
        return { ...s, caret, first, last, viewport: { width: innerWidth, height: innerHeight }, scrollers,
          autocomplete: v ? document.querySelector(".cm6-spike").dataset.pageAutocomplete === "open" : !!popup,
          candidates, events: window.__imeEvents };
      });
      const snapshot = async name => {
        const s = await state();
        journey.snapshots[name] = s;
        fs.writeFileSync(path.join(dir, `${name}.json`), JSON.stringify(s, null, 2) + "\n");
        await browser.saveScreenshot(path.join(dir, `${name}.png`));
        return s;
      };
      const saved = async (name, expected) => {
        await sleep(1300);
        const actual = fs.readFileSync(page, "utf8");
        fs.writeFileSync(path.join(dir, `${name}-saved.md`), actual);
        check(`${name}:saved-file`, actual === expected, { expected, actual });
      };
      const stable = (name, s, expected, caret) => {
        check(`${name}:text-caret-focus`, s.text === expected && s.anchor === caret && s.head === caret && s.focus,
          { expected, caret, text: s.text, anchor: s.anchor, head: s.head, focus: s.focus });
      };
      const visible = s => !!s.caret && s.caret.top >= 0 && s.caret.bottom <= s.viewport.height && s.caret.left >= 0 && s.caret.right <= s.viewport.width
        && s.scrollers.filter(el => /auto|scroll|hidden/.test(el.overflow)).every(el => s.caret.bottom > el.rect.top && s.caret.top < el.rect.bottom);
      execFileSync("ibus", ["engine", "hangul"]);
      await sleep(250);
      await key("F6");
      const before = await snapshot("before");
      await key("g", "k", "s");
      const preedit = await snapshot("preedit");
      const real = preedit.events.some(e => e.type === "compositionstart" && e.trusted)
        && preedit.events.filter(e => e.type === "compositionupdate" && e.trusted).map(e => e.data).join(",").includes("ㅎ,하,한");
      check("trusted-inline-preedit", real, { text: preedit.text, caret: preedit.head });
      if (!real) throw new Error("No trusted native Hangul preedit; infrastructure blocked");
      let insert = "한";
      const cancel = scenario === "escape-cancel" || scenario === "page-autocomplete-cancel";
      if (cancel) await key("Escape");
      else if (scenario === "enter") await key("Return");
      else if (scenario === "backspace") {
        await key("BackSpace");
        const step = await snapshot("backspace-step");
        stable("backspace-step", step, base.slice(0, at) + "하" + base.slice(at), at + 1);
        await key("Shift+space"); insert = "하";
      } else await key("Shift+space");
      const after = await snapshot("after");
      if (cancel) {
        const ends = after.events.filter(e => e.type === "compositionend" && e.trusted);
        const policyCommit = ends.some(e => e.data === "한");
        insert = policyCommit ? "한" : "";
        check("escape-policy", policyCommit || after.text === base, { policy: policyCommit ? "IBus commits preedit on Escape" : "Escape cancels preedit", compositionEnds: ends }, policyCommit);
      }
      const expected = base.slice(0, at) + insert + base.slice(at);
      if (cancel && after.text === null) {
        // IBus commits then forwards Escape; the textarea may end editing.
        // Keep that immediate outcome visible, verify content on disk, and
        // report the last observed native commit caret without inventing one.
        check("escape:editor-disposition", true, { editorOpen: false, focus: after.focus,
          lastCommit: after.events.filter(e => e.type === "compositionend").at(-1)?.after });
      } else if (scenario === "enter") {
        // IBus Hangul commits first, then forwards an ordinary Enter. The
        // textarea is the reference: that same Enter must split immediately.
        stable("after", after, "", 0);
        check("after:new-block", !!after.blockId && after.blockId !== before.blockId, { before: before.blockId, after: after.blockId });
      } else stable("after", after, expected, at + insert.length);
      await saved("after", scenario === "enter" ? `- ${expected}\n-\n- Neighbour stays intact\n` : disk(expected));
      if (scenario.startsWith("page-autocomplete")) {
        let completion = after;
        if (cancel && !after.candidates.includes("한글")) {
          // Ordinary Escape can close the textarea picker after the engine
          // commits, or end editing entirely. Reenter if needed, then a real
          // Backspace must make completion usable again.
          if (!after.focus) {
            await browser.execute(() => document.querySelector(".main-content .ls-block .block-content-wrapper").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
            await browser.$(mode === "cm6" ? ".cm-content" : "textarea.block-editor").waitForExist({ timeout: 10000 });
          }
          await key("BackSpace");
          // Empty page queries intentionally have no textarea candidates.
          // Compose the query again with native input to prove it recovers.
          await key("F6", "g", "k", "s", "Shift+space");
          completion = await snapshot("completion-reopened");
          stable("completion-reopened", completion, "[[한", 3);
          await saved("completion-reopened", disk("[[한"));
        }
        check("completion-usable", completion.autocomplete && completion.candidates.includes("한글"), { open: completion.autocomplete, candidates: completion.candidates });
        // Use each editor's supported mouse completion, not CM6's deferred picker keys.
        const target = await browser.execute(selector => {
          const el = Array.from(document.querySelectorAll(selector)).find(el => el.textContent.includes("한글"));
          const r = el?.getBoundingClientRect();
          return r && { x: r.left + r.width / 2, y: r.top + r.height / 2, label: el.textContent };
        }, mode === "cm6" ? ".cm6-completions button" : ".autocomplete .ac-item");
        if (target) {
          journey.keys.push({ mouse: target, window: ids.at(-1), at: new Date().toISOString() });
          execFileSync("xdotool", ["mousemove", "--window", ids.at(-1), String(Math.round(target.x)), String(Math.round(target.y)), "click", "1"]);
        }
        await sleep(250);
        const completed = await snapshot("completed");
        const completedText = mode === "textarea" ? "[[한글]] " : "[[한글]]";
        stable("completed", completed, completedText, completedText.length);
        await saved("completed", disk("[[한글]]"));
      }
      if (scenario === "enter") {
        await key("Return");
        const second = await snapshot("second-enter");
        stable("second-enter", second, "", 0);
        check("second-enter:new-block", !!second.blockId && second.blockId !== after.blockId, { before: after.blockId, after: second.blockId });
        await saved("second-enter", `- ${expected}\n-\n-\n- Neighbour stays intact\n`);
      }
      if (scenario === "block-start" || scenario === "wrapped-line") {
        check("caret-visible", visible(before) && visible(preedit) && visible(after), { before: before.caret, preedit: preedit.caret, after: after.caret });
        if (scenario === "wrapped-line") check("interior-visual-row", before.caret?.top > before.first?.top && before.caret?.top < before.last?.top,
          { first: before.first, caret: before.caret, last: before.last, scrollBefore: before.scrollers, scrollAfter: after.scrollers });
      }
    } catch (error) {
      check("journey", false, { error: String(error) });
      if (!String(error).includes("screenshot")) await browser?.saveScreenshot(path.join(dir, "failure.png")).catch(() => {});
      report.failureCapsules.push({ commit: report.commit, mode, scenario, expected: "trusted composition, intact text/caret/neighbour and guarded save", observed: String(error.stack || error), classification: "ambiguous", evidence: path.relative(out, dir) });
    } finally {
      // Each semantic failure also gets a compact capsule, even without a throw.
      const failures = journey.checks.filter(c => c.status === "fail");
      if (failures.length && !report.failureCapsules.some(c => c.mode === mode && c.scenario === scenario)) report.failureCapsules.push({ commit: report.commit, mode, scenario,
        expected: "scenario checks pass", observed: failures, classification: "ambiguous", evidence: path.relative(out, dir) });
      journey.status = failures.length ? "fail" : journey.checks.some(c => c.status === "engine-policy") ? "engine-policy" : "pass";
      fs.writeFileSync(path.join(dir, "journey.json"), JSON.stringify(journey, null, 2) + "\n");
      fs.writeFileSync(path.join(out, "report.json"), JSON.stringify(report, null, 2) + "\n");
      try { await browser?.deleteSession(); } catch {}
      try { process.kill(-driver.pid, "SIGTERM"); } catch {}
      fs.closeSync(log);
    }
  }
}
if (report.cases.some(c => c.status === "fail")) process.exitCode = 1;
