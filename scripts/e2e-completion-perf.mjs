// D-20261102-PERF: native key -> completion paint, shared textarea/CM6 path.
// Synthetic fixtures from generate-realistic-graph.mjs; no private graph input.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { scratchProfile, editorKind, editorSelector } from "./lib/native-editor-bench.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const root = path.resolve(import.meta.dirname, "..");
assert.equal(process.platform, "linux", "This native perf harness uses X11 screenshots");
const bulk = path.resolve(process.env.COMPLETION_ARTIFACT_DIR || path.join(root, "target/artifacts/og-perf-completion"));
const arm = process.env.COMPLETION_ARM || "candidate";
const app = path.resolve(process.env.TINE_APP || "target/release/tine");
const revision = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const binarySha256 = crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex");
async function port() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const value = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return value;
}
for (const size of (process.env.COMPLETION_SIZES || "1000,10000,30000").split(",").map(Number)) {
  const graph = path.join(bulk, "graphs", String(size));
  assert.ok(fs.existsSync(path.join(graph, "logseq/config.edn")), "Generate the synthetic fixture first");
  const source = path.join(graph, "pages/Completion Bench.md");
  fs.writeFileSync(source, "- Type here\n");
  fs.writeFileSync(path.join(graph, "pages/Completion Owner.md"), "alias:: AlphaNeedle\nalpha:: two\n- CompletionTarget block\n");
  const out = path.join(bulk, "native", `${arm}-${editorKind}-${size}-${Date.now()}`);
  fs.mkdirSync(out, { recursive: true });
  const profile = scratchProfile(out);
  const fixture = accountHomeFixture(out, out, { ...process.env, ...profile, TINE_GRAPH: graph,
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" });
  const driverPort = await port(), nativePort = await port();
  const log = fs.openSync(path.join(out, "driver.log"), "w");
  let browser, driver, stage = "startup";
  const proof = { revision, binarySha256, app, arm, editorKind, size, samplerVersion: 2, samples: [] };
  const keys = async (...value) => { await browser.keys(value); await browser.releaseActions(); };
  const value = () => browser.execute(selector => {
    const el = document.querySelector(selector);
    return el instanceof HTMLTextAreaElement ? el.value : [...el.querySelectorAll(".cm-line")].map(n => n.textContent).join("\n");
  }, editorSelector);
  async function type(text) {
    for (const c of text) {
      const before = await value();
      await keys(c);
      await browser.waitUntil(async () => await value() !== before, { timeout: 10000 });
    }
  }
  async function reset() {
    await keys("Escape");
    if (!await browser.$(editorSelector).isExisting()) {
      await browser.$(".main-content .ls-block .block-content-wrapper").click();
      await browser.$(editorSelector).waitForDisplayed();
    }
    await browser.$(editorSelector).click();
    await keys("Control", "a"); await keys("Backspace");
    await browser.waitUntil(async () => await value() === "", { timeout: 10000 });
  }
  try {
    driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
      webdriverServerArgs(driverPort, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
      { env: fixture.env, stdio: ["ignore", log, log], detached: true });
    browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error",
      connectionRetryCount: 3, connectionRetryTimeout: 120000, capabilities: tauriCapabilities(app, editorKind) });
    await ensureMainWindow(browser);
    await browser.$(".app-container").waitForExist({ timeout: 120000 });
    await assertAppLease(fixture);
    await openPageByName(browser, "Completion Bench", { entry: "button" });
    await browser.$(".main-content .ls-block .block-content-wrapper").click();
    await browser.$(editorSelector).waitForDisplayed();
    await browser.execute(selector => {
      const probe = window.__completionPerf = { requests: [], reads: [], start: 0 };
      document.addEventListener("keydown", event => {
        if (event.target instanceof Element && event.target.closest(selector) && event.key.length === 1
          && !event.ctrlKey && !event.metaKey && !event.altKey) probe.start = performance.now();
      }, true);
      const fetch = window.fetch;
      window.fetch = function(url, options, ...rest) {
        const command = String(url).split("/").at(-1);
        let read;
        if (["quick_switch", "search", "query_facets", "list_templates"].includes(command)) {
          const args = JSON.parse(options.body || "{}");
          if (command !== "quick_switch" || args.limit) {
            read = { command, args, sent: performance.now() };
            probe.reads.push(read);
          }
        }
        const send = () => {
          const response = fetch.call(this, url, options, ...rest);
          return read ? response.then(answer => { read.reply = performance.now(); return answer; }) : response;
        };
        if (!String(url).endsWith("/quick_switch")) return send();
        const args = JSON.parse(options.body);
        if (!args.limit) return send();
        const request = { query: args.query, start: probe.start, startKind: "keydown", sent: performance.now() };
        probe.requests.push(request);
        return send().then(response => {
          const json = response.json.bind(response);
          response.json = () => json().then(answer => {
            request.reply = performance.now(); request.candidates = answer.map(p => p.name);
            requestAnimationFrame(() => requestAnimationFrame(() => {
              request.labels = [...document.querySelectorAll(".autocomplete .ac-label")].map(n => n.textContent.trim());
              // These generated queries are deliberately absent as exact names.
              // Their Create row identifies the query actually displayed, so
              // an obsolete reply cannot be timed against the previous list.
              request.matchesTypedQuery = request.labels.includes(`Create "${String(args.query).trim()}"`)
                || request.labels.includes(`Create #${String(args.query).trim()}`);
              request.paint = performance.now();
            }));
            return answer;
          });
          return response;
        });
      };
    }, editorSelector);
    stage = "page query extensions";
    for (let repeat = 0; repeat < Number(process.env.COMPLETION_REPEATS || 3); repeat++) {
      await reset(); await type("[[");
      let query = "";
      for (const c of "Topic 34") {
        query += c;
        const from = await browser.execute(() => window.__completionPerf.requests.length);
        await type(c);
        await browser.waitUntil(() => browser.execute((from, query) => window.__completionPerf.requests.slice(from)
          .some(r => r.query === query && r.paint !== undefined && r.matchesTypedQuery), from, query), { timeout: 30000, interval: 20 });
        const sample = await browser.execute((from, query) => window.__completionPerf.requests.slice(from)
          .findLast(r => r.query === query && r.paint !== undefined && r.matchesTypedQuery), from, query);
        assert.ok(sample.labels.length, `No visible choices for ${query}`);
        sample.attempts = await browser.execute((from, query) => window.__completionPerf.requests.slice(from)
          .filter(r => r.query === query).map(({ start, sent, reply, paint, labels, matchesTypedQuery }) => ({ start, sent, reply, paint, matchesTypedQuery, labelCount: labels?.length })), from, query);
        if (process.env.COMPLETION_ASSERT_SINGLE_INPUT === "1") {
          assert.equal(sample.attempts.length, 1, `Duplicate page reads for ${query}`);
          assert.ok(sample.attempts[0].labelCount > 0, `Empty first paint for ${query}`);
        }
        proof.samples.push({ ...sample, repeat, ipcMs: sample.reply - sample.sent,
          totalMs: sample.paint - sample.start, responseToPaintMs: sample.paint - sample.reply });
      }
    }
    if (process.env.COMPLETION_INPUT_CENSUS === "1") {
      stage = "completion input census";
      proof.inputCensus = [];
      for (const scenario of [
        { kind: "tag", opener: "#", chars: "To", expected: [1, 1] },
        { kind: "block", opener: "((", chars: "Co", expected: [1, 1] },
        { kind: "command", opener: "/", chars: "pa", expected: [0, 0] },
        { kind: "property-name", opener: "::", chars: "al", expected: [0, 0] },
        { kind: "property-value", opener: "alpha:: ", chars: "tw", expected: [0, 0] },
      ]) {
        await reset();
        // Keep properties inside a body block. A property-only first block is
        // promoted to the page header, which changes editor identity on save.
        if (scenario.kind.startsWith("property-")) {
          await type("Body"); await keys("Shift", "Enter");
        }
        const sessionFrom = await browser.execute(() => window.__completionPerf.reads.length);
        await type(scenario.opener);
        if (scenario.kind === "block") {
          await browser.waitUntil(() => browser.execute(from => window.__completionPerf.reads.slice(from)
            .some(r => r.command === "search" && r.reply !== undefined), sessionFrom), { timeout: 10000 });
          // A blank block query may settle with the normal No matched blocks
          // status. Wait for its reply to paint without requiring candidate rows.
          await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
        }
        if (["command", "property-name", "property-value"].includes(scenario.kind))
          await browser.$(".autocomplete .ac-item").waitForDisplayed({ timeout: 10000 });
        for (const [index, char] of [...scenario.chars].entries()) {
          const from = await browser.execute(() => window.__completionPerf.reads.length);
          await type(char);
          // Wait through the documented 90ms debounce and the native reply;
          // collect only once the visible list has settled for this key.
          await browser.waitUntil(() => browser.execute((from, kind) => {
            const reads = window.__completionPerf.reads.slice(from);
            return document.querySelectorAll(".autocomplete .ac-item").length > 0
              && (kind !== "block" || reads.some(r => r.command === "search"))
              && (kind !== "tag" || reads.some(r => r.command === "quick_switch"));
          }, from, scenario.kind), { timeout: 10000 });
          await browser.executeAsync(done => setTimeout(() => requestAnimationFrame(() => requestAnimationFrame(done)), 150));
          const reads = await browser.execute(from => window.__completionPerf.reads.slice(from), from);
          proof.inputCensus.push({ kind: scenario.kind, char, reads });
          if (process.env.COMPLETION_ASSERT_SINGLE_INPUT === "1")
            assert.equal(reads.length, scenario.expected[index], `${scenario.kind} reads for ${char}`);
        }
        const sessionReads = await browser.execute(from => window.__completionPerf.reads.slice(from), sessionFrom);
        proof.inputCensus.push({ kind: scenario.kind, sessionReads });
      }
    }
    stage = "authored alias insertion";
    await reset(); await type("[[AlphaN");
    await browser.waitUntil(() => browser.execute(() => {
      const labels = [...document.querySelectorAll(".autocomplete .ac-label")].map(n => n.textContent.trim());
      return labels.includes("AlphaNeedle") && labels.includes('Create "AlphaN"');
    }), { timeout: 10000 });
    execFileSync("import", ["-window", "root", path.join(out, "completion.png")], { env: fixture.env, timeout: 10000 });
    await browser.execute(() => {
      const row = [...document.querySelectorAll(".autocomplete .ac-label")].find(n => n.textContent.trim() === "AlphaNeedle");
      row.closest(".ac-item").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    });
    await browser.waitUntil(async () => (await value()).trim() === "[[AlphaNeedle]]", { timeout: 10000 });
    await keys("Escape");
    await waitForFileText(source, text => text.includes("[[AlphaNeedle]]"));
    proof.outcome = "pass";
  } catch (error) {
    proof.outcome = "fail"; proof.stage = stage; proof.expected = "visible ranked choices and durable authored alias insertion";
    proof.observed = String(error); proof.classification = "ambiguous";
    try { proof.diagnostic = await browser.execute(selector => ({ requests: window.__completionPerf?.requests,
      text: document.querySelector(selector)?.textContent,
      labels: [...document.querySelectorAll(".autocomplete .ac-label")].map(n => n.textContent) }), editorSelector); } catch {}
    try { execFileSync("import", ["-window", "root", path.join(out, "failure.png")], { env: fixture.env, timeout: 10000 }); } catch {}
    throw error;
  } finally {
    fs.writeFileSync(path.join(out, "receipt.json"), JSON.stringify(proof, null, 2));
    console.log(`${proof.outcome}: ${out}`);
    try { await browser?.deleteSession(); } catch {}
    if (driver?.pid) { try { process.kill(-driver.pid, "SIGKILL"); } catch {} }
    fixture.close?.(); fs.closeSync(log);
  }
}
