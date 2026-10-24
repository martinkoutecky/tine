// Nonshipping, real WebKitGTK experiment. All writes are isolated fixtures.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const out = path.resolve("artifacts/spike-cm6", process.env.SPIKE_RUN || "native");
const app = path.resolve(process.env.TINE_APP || "target/release/tine");
const report = { app, commit: "087c303939fa64f932aa830b4b18607ed307529c + working diff", measurements: [], cases: [] };
const source = "TODO 中文😀 **bold** *italic* `code` [[Target]] tail";
const summary = values => {
  const sorted = [...values].sort((a, b) => a - b);
  return { n: sorted.length, p50: sorted[Math.floor(sorted.length * .5)], p95: sorted[Math.floor(sorted.length * .95)], max: sorted.at(-1) };
};
fs.mkdirSync(out, { recursive: true });
for (const size of (process.env.SPIKE_SIZES || "10,1000").split(",").map(Number)) {
  const dir = path.join(out, String(size));
  const graph = path.join(dir, "graph");
  for (const sub of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, sub), { recursive: true });
  for (const sub of ["data", "config", "cache"]) fs.mkdirSync(path.join(dir, sub), { recursive: true });
  const now = new Date();
  const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
  const page = path.join(graph, "journals", `${journal}.md`);
  fs.writeFileSync(page, Array.from({ length: size }, (_, i) => `- ${i === 0 ? source : `Neighbour ${i} with enough text for column navigation`}`).join("\n") + "\n");
  fs.writeFileSync(path.join(graph, "pages", "Target.md"), "- Reference target\n");
  fs.writeFileSync(path.join(graph, "logseq", "config.edn"), "{}\n");
  const log = fs.openSync(path.join(dir, "driver.log"), "w");
  const port = Number(process.env.SPIKE_PORT || 4898);
  const driver = spawn("dbus-run-session", ["--", process.env.TAURI_DRIVER || `${process.env.CARGO_HOME}/bin/tauri-driver`, "--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], {
    detached: true, stdio: ["ignore", log, log], env: { ...process.env, TINE_GRAPH: graph,
      XDG_DATA_HOME: path.join(dir, "data"), XDG_CONFIG_HOME: path.join(dir, "config"), XDG_CACHE_HOME: path.join(dir, "cache"),
      WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" },
  });
  let browser;
  const result = (name, data) => { report.cases.push({ size, name, ...data }); console.log(JSON.stringify(report.cases.at(-1))); };
  try {
    await sleep(1500);
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
      connectionRetryTimeout: 120000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
    await ensureMainWindow(browser, { what: "CM6 spike main window" });
    await browser.$(".page-title").waitForExist({ timeout: 60000 });
    const enter = async () => {
      await browser.execute(() => document.querySelector(".main-content .ls-block .block-content-wrapper").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
      await browser.$("textarea.block-editor, .cm-content").waitForExist({ timeout: 10000 });
    };
    const setText = async (text, anchor = text.length, head = anchor) => browser.execute((value, a, h) => {
      const host = document.querySelector(".cm6-spike");
      if (host) { const v = host.cm6View; v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value }, selection: { anchor: a, head: h } }); v.focus(); }
      else { const e = document.querySelector("textarea.block-editor"); e.value = value; e.setSelectionRange(a, h); e.dispatchEvent(new InputEvent("input", { bubbles: true, data: value, inputType: "insertText" })); e.focus(); }
    }, text, anchor, head);
    const state = () => browser.execute(() => {
      const host = document.querySelector(".cm6-spike");
      if (host) { const v = host.cm6View; return { text: v.state.doc.toString(), from: v.state.selection.main.from, to: v.state.selection.main.to, id: v.contentDOM.dataset.cm6Editor, chips: [...host.querySelectorAll(".cm6-chip")].map(e => e.textContent), composing: v.composing }; }
      const e = document.querySelector("textarea.block-editor"); return e ? { text: e.value, from: e.selectionStart, to: e.selectionEnd } : null;
    });
    // Alternate order on large page; each mode is compiled into the SAME binary.
    for (const mode of size === 10 ? ["textarea", "cm6"] : ["cm6", "textarea"]) {
      await browser.execute(mode => { sessionStorage.setItem("tine-spike-cm6", mode === "cm6" ? "1" : "0"); location.reload(); }, mode);
      await browser.$(".page-title").waitForExist({ timeout: 60000 });
      await sleep(1000);
      await enter();
      if ((await browser.$(".cm-content").isExisting()) !== (mode === "cm6")) throw new Error(`wrong editor for ${mode}`);
      for (let run = 0; run < Number(process.env.SPIKE_RUNS ?? 3); run++) {
        await setText(source);
        await sleep(250);
        await browser.execute(() => {
          const p = window.__cmBench = { active: true, previous: 0, frames: [], paint: [], inputs: 0 };
          p.listener = event => {
            if (!event.target.matches("textarea.block-editor, .cm-content")) return;
            const at = performance.now(); p.inputs++;
            requestAnimationFrame(() => setTimeout(() => { if (p.active) p.paint.push(performance.now() - at); }, 0));
          };
          document.addEventListener("beforeinput", p.listener, true);
          const tick = at => { if (!p.active) return; if (p.previous) p.frames.push(at - p.previous); p.previous = at; requestAnimationFrame(tick); };
          requestAnimationFrame(tick);
        });
        const actions = [];
        for (let i = 0; i < 100; i++) actions.push({ type: "keyDown", value: "x" }, { type: "keyUp", value: "x" }, { type: "pause", duration: 33 });
        await browser.performActions([{ type: "key", id: "spike-typing", actions }]);
        await browser.releaseActions();
        await sleep(100);
        const raw = await browser.execute(() => {
          const p = window.__cmBench; p.active = false; document.removeEventListener("beforeinput", p.listener, true);
          return { inputs: p.inputs, paint: p.paint, frames: p.frames, focused: document.activeElement?.matches("textarea.block-editor, .cm-content") };
        });
        fs.writeFileSync(path.join(dir, `${mode}-${run}.json`), JSON.stringify(raw));
        const entry = { size, mode, run, inputToPaintMs: summary(raw.paint), framesOver50Ms: raw.frames.filter(x => x > 50).length, maxFrameMs: Math.max(...raw.frames), inputs: raw.inputs, focused: raw.focused };
        report.measurements.push(entry); console.log(JSON.stringify(entry));
        if (raw.inputs !== 100 || !raw.focused || (await state()).text !== source + "x".repeat(100)) throw new Error("native typing lost input/focus");
      }
      await setText(source);
      await browser.saveScreenshot(path.join(dir, `${mode}-preview.png`));
      if (mode !== "cm6") { await browser.keys("Escape"); continue; }
      result("preview", await state());
      await setText(source, 0);
      result("reveal-task-boundary", { pass: !(await state()).chips.includes("TODO") });
      await setText(source, source.indexOf("[["));
      result("reveal-link-boundary", { pass: !(await state()).chips.includes("Target") });
      await browser.saveScreenshot(path.join(dir, "cm6-reveal-source.png"));
      await setText(source);
      // Physical pointer click on styled bold; selected source should be within it.
      await browser.$(".cm6-bold").click();
      const clicked = await state();
      result("click-bold", { ...clicked, pass: clicked.from >= source.indexOf("**bold**") && clicked.from <= source.indexOf("**bold**") + 8 });
      await setText(source);
      await browser.performActions([{ type: "key", id: "spike-selection", actions: [
        { type: "keyDown", value: "\uE008" },
        ...Array.from({ length: 4 }, () => [{ type: "keyDown", value: "\uE012" }, { type: "keyUp", value: "\uE012" }]).flat(),
        { type: "keyUp", value: "\uE008" },
      ] }]);
      await browser.releaseActions();
      result("native-selection", { pass: (await state()).to - (await state()).from === 4, observed: await state() });
      await setText(source, 5, 11);
      await browser.keys("z");
      result("replace-selection", { pass: (await state()).text === source.slice(0, 5) + "z" + source.slice(11) });
      await browser.keys(["Control", "z"]);
      result("undo", { pass: (await state()).text === source });
      await browser.keys(["Control", "Shift", "z"]);
      result("redo", { pass: (await state()).text === source.slice(0, 5) + "z" + source.slice(11) });
      await setText(source, 6);
      const id = (await state()).id;
      result("vertical-geometry", await browser.execute(() => {
        const v = document.querySelector(".cm6-spike").cm6View;
        const pos = v.state.selection.main.head;
        const r = v.coordsAtPos(pos);
        const line = v.lineBlockAt(pos);
        return { head: pos, length: v.state.doc.length, coords: r ? { top: r.top, bottom: r.bottom } : null,
          documentTop: v.documentTop, line: { from: line.from, to: line.to, top: line.top, bottom: line.bottom },
          lineHeight: v.defaultLineHeight, up: v.moveVertically(v.state.selection.main, false).head,
          down: v.moveVertically(v.state.selection.main, true).head };
      }));
      await browser.keys("ArrowDown"); await sleep(100);
      const down = await state();
      result("arrow-down", { pass: down?.id !== id && down?.from === 6, observed: down });
      await browser.keys("ArrowUp"); await sleep(100);
      result("arrow-up", { pass: (await state())?.id === id && (await state())?.from === 6, observed: await state() });
      const wrapped = "wrapped words ".repeat(80);
      await setText(wrapped, 5);
      await browser.keys("ArrowDown");
      result("wrapped-interior-down", { pass: (await state())?.id === id && (await state())?.from > 5 && (await state())?.from < wrapped.length, observed: await state() });
      await setText(wrapped);
      const lastStart = await browser.execute(() => {
        const v = document.querySelector(".cm6-spike").cm6View;
        return v.moveToLineBoundary(v.state.selection.main, false).head;
      });
      await setText(wrapped, lastStart + 4);
      await browser.keys("ArrowDown"); await sleep(100);
      result("wrapped-last-row-down", { pass: (await state())?.id !== id && (await state())?.from === 4, observed: await state() });
      await browser.keys("ArrowUp"); await sleep(100);
      result("wrapped-first-row-up", { pass: (await state())?.id === id && (await state())?.from === lastStart + 4, observed: await state(), expectedOffset: lastStart + 4 });
      await setText(source, 12);
      await browser.keys("Enter"); await sleep(100);
      result("split", { pass: (await state())?.text === source.slice(12), observed: await state() });
      await browser.keys("Backspace"); await sleep(100);
      result("merge", { pass: (await state())?.text === source, observed: await state() });
      await setText("[[");
      result("autocomplete-opening", { pass: await browser.execute(() => document.querySelector(".cm6-spike").dataset.pageAutocomplete === "open") });
      await browser.saveScreenshot(path.join(dir, "cm6-autocomplete.png"));
      await setText("compose ");
      // Mirror existing IME harness with real contenteditable DOM mutation,
      // not EditorView.dispatch. This tests CM6's DOM observer/composition path.
      await browser.execute(() => {
        const el = document.querySelector(".cm-content"); el.focus();
        el.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data: "" }));
        for (const key of ["Enter", "Escape"]) el.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key, keyCode: 229, isComposing: true }));
        document.execCommand("insertText", false, "中文");
        el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertCompositionText", data: "中文", isComposing: true }));
        el.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "中文" }));
      });
      await sleep(250);
      result("synthetic-CJK-composition", { pass: (await state()).text === "compose 中文" && !(await state()).composing, observed: await state(), limitation: "No OS IME candidate window tested" });
      await setText(source + " saved");
      await browser.keys("Escape");
      await sleep(1200);
      result("escape-and-guarded-save", { pass: !(await browser.$(".cm-content").isExisting()) && fs.readFileSync(page, "utf8").includes(source + " saved") });
      if (size === 10) {
        await browser.execute(() => document.querySelector(".main-content .ls-block .bullet-container").dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, shiftKey: true })));
        await browser.$(".rs-item .ls-block").waitForExist({ timeout: 10000 });
        await browser.execute(() => document.querySelector(".rs-item .ls-block .block-content-wrapper").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
        await browser.$(".rs-item .cm-content").waitForExist({ timeout: 10000 });
        result("duplicate-sidebar-instance", await browser.execute(() => ({
          pass: document.querySelectorAll(".cm-editor").length === 1 && !document.querySelector(".main-content .cm-editor"),
          cmViews: document.querySelectorAll(".cm-editor").length,
          mainRendered: !!document.querySelector(".main-content .ls-block .block-content"),
        })));
        await browser.keys("Escape");
        await browser.execute(() => document.querySelector(".rs-item .rs-close").click());
      }
    }
  } catch (error) {
    report.error = String(error.stack || error);
    report.failureCapsule = { testedCommit: report.commit, journey: "CM6 native spike", expected: "Native typing, decorated caret, structural commands and isolated persisted edits", observed: String(error), classification: "ambiguous", screenshot: `${size}/failure.png` };
    await browser?.saveScreenshot(path.join(dir, "failure.png")).catch(() => {});
    if (browser) fs.writeFileSync(path.join(dir, "failure.html"), await browser.getPageSource().catch(() => "unavailable"));
    throw error;
  } finally {
    fs.writeFileSync(path.join(out, "report.json"), JSON.stringify(report, null, 2) + "\n");
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-driver.pid, "SIGTERM"); } catch {}
    fs.closeSync(log);
  }
}
