// GH #944 native typing journey on a realistic 2000-page graph.
// Section disappearance fails; incidental movement is only reported.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';
import { remote } from 'webdriverio';
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from './e2e-capabilities.mjs';
import { ensureMainWindow } from './lib/e2e-main-window.mjs';
import { openJournals, openPageByName } from './lib/e2e-navigation.mjs';
import { generateRealisticGraph } from './generate-realistic-graph.mjs';
import { installAgendaProbe, summarizeKeys } from './lib/gh944-probe.mjs';
import { accountHomeFixture, assertAppLease } from './lib/e2e-account-home.mjs';
import { ensurePrivateSessionBus } from './lib/e2e-session-bus.mjs';

ensurePrivateSessionBus();
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const app = path.resolve(process.env.TINE_APP || path.join(root, 'target/debug/tine'));
const out = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, 'target/artifacts/fix-944/native'));
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, 'run-'));
const graph = path.join(run, 'graph');
const manifest = await generateRealisticGraph({ root: graph, pages: 1500, journals: 500 });
for (const dir of ['pages', 'journals', 'logseq']) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ['home', 'data', 'config', 'cache', 'state', 'appdata', 'localappdata', 'app-tmp']) fs.mkdirSync(path.join(run, dir));
const pad = n => String(n).padStart(2, '0');
const date = offset => { const d = new Date(); d.setDate(d.getDate() + offset); return d; };
const stem = d => `${d.getFullYear()}_${pad(d.getMonth() + 1)}_${pad(d.getDate())}`;
const org = d => `<${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${d.toLocaleString('en-US', { weekday: 'short' })}>`;
fs.writeFileSync(path.join(graph, 'logseq/config.edn'), '{:tine/guide-announced? true}\n');
fs.writeFileSync(path.join(graph, 'journals', `${stem(date(0))}.md`),
  `- today outside agenda\n- TODO today inside agenda\n  SCHEDULED: ${org(date(0))}\n- today last\n`);
for (let ago = 1; ago <= 8; ago++) {
  const lines = Array.from({ length: 12 }, (_, i) => `- day ${ago} row ${i}\n  - child a\n  - child b`);
  fs.writeFileSync(path.join(graph, 'journals', `${stem(date(-ago))}.md`), lines.join('\n') + '\n');
}
const tasks = [];
for (let offset = -2; offset <= 5; offset++) {
  tasks.push(`- TODO scheduled ${offset}\n  SCHEDULED: ${org(date(offset))}`);
  if (offset >= 0) tasks.push(`- TODO deadline ${offset}\n  DEADLINE: ${org(date(offset))}`);
}
fs.writeFileSync(path.join(graph, 'pages/Tasks.md'), tasks.join('\n') + '\n');
// Windows resolves app data through the known-folder API; overriding USERPROFILE/HOME makes it fail ("unknown path"),
// so only APPDATA/LOCALAPPDATA are redirected there, as in e2e-windows-smoke.mjs and e2e-gh745-windows-probe.mjs.
const homeEnv = process.platform === 'win32' ? {} : { HOME: path.join(run, 'home'), USERPROFILE: path.join(run, 'home') };
const env = { ...process.env, TINE_GRAPH: graph, ...homeEnv,
  XDG_DATA_HOME: path.join(run, 'data'), XDG_CONFIG_HOME: path.join(run, 'config'),
  XDG_CACHE_HOME: path.join(run, 'cache'), XDG_STATE_HOME: path.join(run, 'state'),
  APPDATA: path.join(run, 'appdata'), LOCALAPPDATA: path.join(run, 'localappdata'), TMPDIR: path.join(run, 'app-tmp'),
  WEBKIT_DISABLE_DMABUF_RENDERER: '1', WEBKIT_DISABLE_COMPOSITING_MODE: '1', LIBGL_ALWAYS_SOFTWARE: '1', GDK_BACKEND: 'x11' };
const owner = accountHomeFixture(run, run, env);
Object.assign(env, owner.env);
const port = Number(process.env.E2E_DRIVER_PORT || 4944), nativePort = Number(process.env.E2E_NATIVE_PORT || 4945);
const results = { commit: process.env.TINE_SOURCE_REVISION || 'working-tree', app, graph, platform: process.platform,
  manifest, cadenceMs: 450, phases: [], transitions: [], captureFailures: [], sampling: 'every rAF plus before/keydown/after; section presence and rows' };
let browser, target, driver, log;
let stage = 'startup';
let captureError;
async function capture(name) {
  try { await browser.saveScreenshot(path.join(run, name)); }
  catch (error) {
    results.captureFailures.push({ stage, name, error: String(error) });
    captureError ??= error;
  }
}
try {
  target = await startWebdriverApplication(app, env, nativePort);
  log = fs.openSync(path.join(run, 'driver.log'), 'w');
  const td = process.env.TAURI_DRIVER || (process.platform === 'win32' ? 'msedgedriver.exe' : 'tauri-driver');
  driver = spawn(td, webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || '/usr/bin/WebKitWebDriver'),
    { env: target.env, stdio: ['ignore', log, log], detached: process.platform !== 'win32' });
  const deadline = Date.now() + 15_000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (driver.exitCode !== null || Date.now() > deadline) throw new Error('WebDriver server did not start');
    await sleep(100);
  }
  browser = await remote({ hostname: '127.0.0.1', port, path: '/', logLevel: 'error', connectionRetryCount: 1,
    connectionRetryTimeout: 60_000, capabilities: tauriCapabilities(app, 'default', process.platform, target.debuggerAddress)
  });
  await ensureMainWindow(browser);
  try { await browser.setWindowRect(0, 0, 1280, 800); }
  catch (error) { results.windowRectError = String(error); }
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector('.nav-item')), { timeout: 60_000 });
  await openJournals(browser);
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll('.agenda-block .ls-block')]
    .some(row => row.textContent.includes('today inside agenda'))), { timeout: 30_000 });
  await assertAppLease(owner);
  results.environment = await browser.execute(() => ({ userAgent: navigator.userAgent, dpr: devicePixelRatio,
    width: innerWidth, height: innerHeight, date: new Date().toString(), url: location.href }));
  const appUrl = new URL(results.environment.url);
  if (appUrl.protocol !== 'tauri:' && appUrl.hostname !== 'tauri.localhost') throw new Error(`Expected embedded app assets, got ${appUrl.origin}`);
  await capture('00-start.png');
  const todayTitle = await browser.execute(() => document.querySelector('.page-section .page-title').textContent.trim());
  for (const phase of ['expanded', 'collapsed', 'routed-expanded', 'routed-collapsed']) {
    stage = phase;
    if (phase === 'routed-expanded') {
      // The feed's first h1 already has today's title. Leave it before using
      // the shared helper, whose same-title fast path otherwise keeps the feed.
      await openPageByName(browser, 'Tasks');
      await openPageByName(browser, todayTitle);
    }
    if (phase.startsWith('routed-')) await browser.waitUntil(() => browser.execute(today => {
      const pages = [...document.querySelectorAll('.main-content .page-section[data-page-name]')];
      return pages.length === 1 && pages[0].dataset.pageName === today;
    }, todayTitle), { timeout: 30_000, timeoutMsg: 'today did not open as an individual routed page' });
    const collapsed = phase.endsWith('collapsed');
    await browser.waitUntil(() => browser.execute(() => !!document.querySelector('.agenda-block .query-collapse')), { timeout: 30_000 });
    await browser.execute(installAgendaProbe);
    await browser.execute(collapsed => {
      const toggle = document.querySelector('.agenda-block .query-collapse');
      if (toggle.classList.contains('collapsed') !== collapsed) toggle.click();
    }, collapsed);
    await browser.waitUntil(() => browser.execute(collapsed => {
      const toggle = document.querySelector('.agenda-block .query-collapse');
      return !!toggle && toggle.classList.contains('collapsed') === collapsed;
    }, collapsed), { timeout: 30_000 });
    const transition = await browser.execute(() => {
      window.__gh944.stop();
      return { frames: window.__gh944.frames, missingListFrames: window.__gh944.frames.filter(frame => frame.top === null).length };
    });
    results.transitions.push({ phase, ...transition });
    const needle = 'today outside agenda';
    const point = await browser.execute(needle => {
      const content = [...document.querySelectorAll('.page-section .block-content')]
        .find(el => !el.closest('.agenda-block') && el.textContent.includes(needle));
      if (!content) throw new Error(`missing journal block ${needle}`);
      // A scheduled row's center can land on its date chip. Click the text
      // itself, using its range rather than the whole row's bounding box.
      const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
      while (walker.nextNode()) {
        const at = walker.currentNode.textContent.indexOf(needle);
        if (at < 0) continue;
        const range = document.createRange();
        range.setStart(walker.currentNode, at);
        range.setEnd(walker.currentNode, at + needle.length);
        const box = range.getBoundingClientRect();
        return { x: Math.round(box.left + box.width / 2), y: Math.round(box.top + box.height / 2) };
      }
      throw new Error(`missing text range ${needle}`);
    }, needle);
    await browser.performActions([{ type: 'pointer', id: 'gh944-mouse', parameters: { pointerType: 'mouse' }, actions: [
      { type: 'pointerMove', origin: 'viewport', duration: 0, ...point },
      { type: 'pointerDown', button: 0 }, { type: 'pointerUp', button: 0 },
    ] }]);
    await browser.releaseActions();
    await browser.waitUntil(() => browser.execute(needle => document.activeElement?.matches('textarea.block-editor')
      && document.activeElement.value.includes(needle), needle), { timeout: 10_000 });
    await browser.keys('End');
    const setup = await browser.execute(installAgendaProbe);
    const listed = await browser.execute(() => {
      const id = document.activeElement.closest('.ls-block').dataset.blockId;
      return [...document.querySelectorAll('.agenda-block .ls-block')].some(row => row.dataset.blockId === id);
    });
    if (listed) throw new Error(`unexpected agenda membership for ${phase} editor`);
    const keys = [];
    for (const [i, char] of [...' alpha beta gamma delta epsilon zeta theta'].entries()) {
      keys.push(char);
      if ([9, 19, 29].includes(i)) keys.push('Enter');
    }
    for (const [index, key] of keys.entries()) {
      await browser.execute(key => window.__gh944.begin(key), key);
      await browser.keys(key);
      // Deliberate human cadence / observation window, never a readiness wait.
      await sleep(results.cadenceMs);
      await browser.execute(() => window.__gh944.finish());
      if (index === keys.length - 1 && !collapsed) {
        // Cover folding after typing with the same presence sampler.
        await browser.$('.agenda-block .query-collapse').click();
        // Observe the transition for two actual layout frames, not a readiness sleep.
        await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(() => done(null))));
      }
    }
    const recorded = await browser.execute(() => ({ keys: window.__gh944.stop(), frames: window.__gh944.frames, mutations: window.__gh944.allMutations }));
    const raw = recorded.keys;
    const summary = summarizeKeys(raw);
    const pageNames = await browser.execute(() => [...document.querySelectorAll('.main-content .page-section[data-page-name]')].map(page => page.dataset.pageName));
    results.phases.push({ phase, pageNames, foldedAfterTyping: !collapsed, enter: 'Enter (new blocks)', setup, listed, summary, keys: raw,
      frames: recorded.frames, missingListFrames: recorded.frames.filter(frame => frame.top === null).length,
      allMutations: recorded.mutations });
    if (summary.some(k => k.events !== 1 || k.frames < 2)) throw new Error('missing key event or animation-frame samples');
    if (raw.some(k => k.frames.some(f => f.editorHeight === null))) throw new Error('active editor disappeared during sampling');
    await capture(`01-${phase}.png`);
    await browser.keys('Escape');
    fs.writeFileSync(path.join(out, 'gh944-probe.json'), JSON.stringify(results, null, 2));
  }
  console.log('GH944 PROBE VERDICT ' + results.phases.map(p => [
    `${p.phase}: max list-top excursion=${Math.max(...p.summary.map(k => k.maxTopExcursion)).toFixed(2)}px`,
    `frames>0.5px=${p.summary.reduce((n, k) => n + k.framesOverHalfPixel, 0)}`,
    `mutation batches/key=[${p.summary.map(k => k.mutationBatches).join(',')}]`,
    `transient overshoot=${Math.max(...p.summary.map(k => k.maxTransientOvershoot)).toFixed(2)}px`,
    `one-frame editor dips=${p.summary.reduce((n, k) => n + k.oneFrameEditorDips, 0)}`,
    `missing-list frames=${p.summary.reduce((n, k) => n + k.missingListFrames, 0)}`,
  ].join(', ')).join('; '));
  if (results.phases.some(p => p.missingListFrames) || results.transitions.some(p => p.missingListFrames)) throw new Error('GH #944: Scheduled & Deadline disappeared while typing or folding');
  results.semanticPassed = true;
  if (captureError) throw captureError;
} catch (error) {
  if (!captureError) try { await browser?.saveScreenshot(path.join(run, 'failure.png')); } catch {}
  try { results.failureState = await browser?.execute(() => ({ active: document.activeElement?.outerHTML.slice(0, 1000),
    agenda: document.querySelector('.agenda-block')?.outerHTML,
    probeFrames: window.__gh944?.frames,
    editors: [...document.querySelectorAll('textarea.block-editor')].map(editor => editor.value),
    dialogs: [...document.querySelectorAll('[role="dialog"]')].map(dialog => dialog.textContent.slice(0, 1000)) })); } catch {}
  results.failure = { stage, journey: 'GH944 journal agenda typing', expected: 'section stays present while typing, every key and frame recorded',
    observed: String(error), classification: String(error).includes('Scheduled & Deadline disappeared') || results.phases.some(phase => phase.missingListFrames) ? 'product'
      : results.semanticPassed && captureError ? 'infrastructure (screenshot endpoint; semantic checks passed)'
        : 'ambiguous (setup/input/infrastructure; inspect driver.log and screenshot)' };
  throw error;
} finally {
  fs.writeFileSync(path.join(out, 'gh944-probe.json'), JSON.stringify(results, null, 2));
  try { await browser?.execute(() => window.__gh944?.stop()); } catch {}
  try { await browser?.deleteSession(); } catch {}
  if (driver) {
    if (process.platform === 'win32') spawnSync('taskkill', ['/PID', String(driver.pid), '/T', '/F'], { stdio: 'ignore' });
    else try { process.kill(-driver.pid, 'SIGKILL'); } catch {}
  }
  stopWebdriverApplication(target);
  if (log !== undefined) fs.closeSync(log);
  owner.close?.();
}
