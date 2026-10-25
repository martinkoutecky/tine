// Real X input -> GTK IBus -> WebKitGTK, in fixture graphs only.
// source scripts/env.sh; xvfb-run -a -s "-screen 0 1920x1080x24" \
//   dbus-run-session -- bash scripts/spike-cm6-ime-env.sh
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const out = process.env.SPIKE_IME_OUT;
if (!out || !process.env.IBUS_ADDRESS) throw new Error("Use spike-cm6-ime-env.sh inside a private X/DBus session");
const report = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), cases: [], engine: "IBus Hangul 1.5.4, two-set keyboard", app: path.resolve(process.env.TINE_APP || "target/release/tine") };
const key = async (...keys) => { execFileSync("xdotool", ["key", "--clearmodifiers", "--delay", "100", ...keys]); await sleep(300); };
const modeList = (process.env.SPIKE_IME_MODES || "textarea,cm6").split(",");
for (const [modeIndex, mode] of modeList.entries()) {
  const dir = path.join(out, mode);
  const graph = path.join(dir, "graph");
  for (const sub of ["journals", "pages", "logseq"]) fs.mkdirSync(path.join(graph, sub), { recursive: true });
  for (const sub of ["data", "config", "cache"]) fs.mkdirSync(path.join(dir, sub), { recursive: true });
  const now = new Date();
  const stamp = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
  const page = path.join(graph, "journals", `${stamp}.md`);
  fs.writeFileSync(page, "- IME fixture\n- Neighbour stays intact\n");
  fs.writeFileSync(path.join(graph, "pages", "한글.md"), "- Candidate page\n");
  fs.writeFileSync(path.join(graph, "logseq", "config.edn"), "{}\n");
  const log = fs.openSync(path.join(dir, "driver.log"), "w");
  // Separate listeners avoid reusing a just-closed driver socket.
  const port = Number(process.env.SPIKE_IME_PORT || 4908) + modeIndex * 2;
  const driver = spawn(process.env.TAURI_DRIVER || `${process.env.CARGO_HOME}/bin/tauri-driver`, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], {
    detached: true, stdio: ["ignore", log, log], env: { ...process.env, TINE_GRAPH: graph,
      XDG_DATA_HOME: path.join(dir, "data"), XDG_CONFIG_HOME: path.join(dir, "config"), XDG_CACHE_HOME: path.join(dir, "cache"),
      WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" },
  });
  let browser;
  const result = (scenario, pass, observed) => {
    report.cases.push({ mode, scenario, pass, observed });
    console.log(JSON.stringify(report.cases.at(-1)));
  };
  try {
    await sleep(1500);
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
      connectionRetryTimeout: 120000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: report.app } } });
    await ensureMainWindow(browser, { what: "real IME spike" });
    await browser.$(".page-title").waitForExist({ timeout: 60000 });
    await browser.execute(mode => { sessionStorage.setItem("tine-spike-cm6", mode === "cm6" ? "1" : "0"); location.reload(); }, mode);
    await browser.$(".page-title").waitForExist({ timeout: 60000 });
    await sleep(750);
    const enter = async () => {
      await browser.execute(() => document.querySelector(".main-content .ls-block .block-content-wrapper").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
      await browser.$(mode === "cm6" ? ".cm-content" : "textarea.block-editor").waitForExist({ timeout: 10000 });
    };
    await enter();
    // Observe real events; never synthesize composition or text input.
    await browser.execute(() => {
      window.__imeEvents = [];
      for (const type of ["compositionstart", "compositionupdate", "compositionend", "beforeinput", "input", "keydown"])
        document.addEventListener(type, e => window.__imeEvents.push({ type, data: e.data, key: e.key, isComposing: e.isComposing, trusted: e.isTrusted, inputType: e.inputType }), true);
    });
    const state = () => browser.execute(() => {
      const host = document.querySelector(".cm6-spike");
      const e = document.querySelector("textarea.block-editor");
      return { text: host ? host.cm6View.state.doc.toString() : e?.value ?? null,
        focus: document.activeElement?.matches("textarea.block-editor, .cm-content"),
        composing: host?.cm6View.compositionStarted ?? null,
        autocomplete: host ? host.dataset.pageAutocomplete : !!document.querySelector(".autocomplete"),
        events: window.__imeEvents };
    });
    const reset = async (value, at = value.length) => {
      // Switch to Latin before fixture reset to finish any previous preedit.
      execFileSync("ibus", ["engine", "xkb:us::eng"]);
      await sleep(150);
      if (!(await browser.$("textarea.block-editor, .cm-content").isExisting())) await enter();
      await browser.execute((text, anchor) => {
        const host = document.querySelector(".cm6-spike");
        if (host) { const v = host.cm6View; v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: text }, selection: { anchor } }); v.focus(); }
        else { const e = document.querySelector("textarea.block-editor"); e.value = text; e.setSelectionRange(anchor, anchor); e.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text })); e.focus(); }
        window.__imeEvents.length = 0;
      }, value, at);
      execFileSync("ibus", ["engine", "hangul"]);
      await sleep(250);
      await key("F6"); // Engine's configured ON key; Escape may have left Latin mode.
    };
    // Focus the real app window (no window manager is required for windowfocus).
    const ids = execFileSync("xdotool", ["search", "--onlyvisible", "--class", "tine"], { encoding: "utf8" }).trim().split("\n");
    execFileSync("xdotool", ["windowfocus", "--sync", ids.at(-1)]);
    const compose = async () => { await key("g", "k", "s"); };
    for (const [scenario, base, at] of [["commit", "commit ", 7], ["escape-cancel", "cancel ", 7], ["enter", "enter ", 6], ["backspace", "backspace ", 10], ["page-autocomplete", "[[", 2], ["block-start", " tail", 0], ["wrapped-line", "wrapped words ".repeat(65), 700]]) {
      if (process.env.SPIKE_IME_SCENARIOS && !process.env.SPIKE_IME_SCENARIOS.split(",").includes(scenario)) continue;
      try {
        await reset(base, at);
        await compose();
        const preedit = await state();
        await browser.saveScreenshot(path.join(dir, `${scenario}-preedit.png`));
        const real = preedit.events.some(e => e.type === "compositionstart" && e.trusted);
        result(`${scenario}:real-preedit`, real, preedit);
        if (!real) throw new Error("No trusted OS composition in WebKitGTK; GTK Entry control succeeds. Remaining IME scenarios are blocked, not product failures.");
        if (scenario === "escape-cancel") await key("Escape");
        else if (scenario === "enter") await key("Return");
        else if (scenario === "backspace") { await key("BackSpace"); await key("Shift+space"); }
        else await key("Shift+space"); // Commit by switching mode; do not navigate to another block.
        const after = await state();
        await browser.saveScreenshot(path.join(dir, `${scenario}-after.png`));
        const insert = scenario === "escape-cancel" ? "" : scenario === "backspace" ? "하" : "한";
        const expected = base.slice(0, at) + insert + base.slice(at);
        result(scenario, after.text === expected && after.focus, { expected, ...after });
        if (scenario === "page-autocomplete") result("page-autocomplete:open-after-commit", after.autocomplete === true || after.autocomplete === "open", after);
        await sleep(1200);
        const saved = fs.readFileSync(page, "utf8");
        fs.writeFileSync(path.join(dir, `${scenario}-saved.md`), saved);
        const expectedDisk = `- ${expected}\n- Neighbour stays intact\n`;
        result(`${scenario}:disk-exactly-once`, saved === expectedDisk, { expected: expectedDisk, saved });
        if (after.text !== expected || !after.focus || saved !== expectedDisk) throw new Error(`${scenario}: observed editor/save mismatch; stop this editor before later scenarios can inherit a changed outline`);
      } catch (error) {
        result(scenario, false, { error: String(error) });
        throw error;
      }
    }
  } catch (error) {
    report.failureCapsules ??= [];
    report.failureCapsules.push({ commit: report.commit, mode, journey: "real OS IME", expected: "trusted composition, stable edit and exactly-once guarded save", observed: String(error.stack || error), classification: "ambiguous", screenshot: `${mode}/failure.png` });
    await browser?.saveScreenshot(path.join(dir, "failure.png")).catch(() => {});
  } finally {
    fs.writeFileSync(path.join(out, "report.json"), JSON.stringify(report, null, 2) + "\n");
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-driver.pid, "SIGTERM"); } catch {}
    fs.closeSync(log);
  }
}
if (report.failureCapsules?.length || report.cases.some(c => !c.pass)) process.exitCode = 1;
