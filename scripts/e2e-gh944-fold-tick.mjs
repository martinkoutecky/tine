// GH #944 comment 11: on the Journals feed, folding/unfolding a block and ticking a task must not move
// the block under the pointer while the page still scrolls. Per-frame target top, scrollTop, section heights and
// backend command counts for every route in (bullet-side toggle click / keyboard; checkbox click / Ctrl+Enter),
// in today's journal and in an older day. Movement because the page no longer needs to scroll is C837 (by design).
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';
import { remote } from 'webdriverio';
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from './e2e-capabilities.mjs';
import { ensureMainWindow } from './lib/e2e-main-window.mjs';
import { openJournals } from './lib/e2e-navigation.mjs';
import { generateRealisticGraph } from './generate-realistic-graph.mjs';
import { installFoldProbe, summarizeFold, classifyFold } from './lib/gh944-fold-probe.mjs';
import { accountHomeFixture, assertAppLease } from './lib/e2e-account-home.mjs';
import { ensurePrivateSessionBus } from './lib/e2e-session-bus.mjs';

ensurePrivateSessionBus();
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const app = path.resolve(process.env.TINE_APP || path.join(root, 'target/debug/tine'));
const out = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, 'target/artifacts/fix-944-fold/native'));
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, 'run-'));
const graph = path.join(run, 'graph');
// A realistic graph (hub pages with backlinks, tags, tasks, queries in recent journals) under the fixed days below,
// so the feed's Linked References and query sections are as heavy as a real graph's.
const manifest = await generateRealisticGraph({ root: graph, pages: Number(process.env.E2E_PAGES || 300), journals: Number(process.env.E2E_JOURNALS || 120) });
for (const dir of ['pages', 'journals', 'logseq']) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ['home', 'data', 'config', 'cache', 'state', 'appdata', 'localappdata', 'app-tmp']) fs.mkdirSync(path.join(run, dir));
const pad = n => String(n).padStart(2, '0');
const date = offset => { const d = new Date(); d.setDate(d.getDate() + offset); return d; };
const stem = d => `${d.getFullYear()}_${pad(d.getMonth() + 1)}_${pad(d.getDate())}`;
const org = d => `<${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${d.toLocaleString('en-US', { weekday: 'short' })}>`;
const ordinal = n => `${n}${n % 100 >= 11 && n % 100 <= 13 ? 'th' : ({ 1: 'st', 2: 'nd', 3: 'rd' }[n % 10] ?? 'th')}`;
const title = d => `${d.toLocaleString('en-US', { month: 'short' })} ${ordinal(d.getDate())}, ${d.getFullYear()}`;
// The reporter's "query window (ToDos)" is a journals default query above the built-in Scheduled & Deadline list.
fs.writeFileSync(path.join(graph, 'logseq/config.edn'),
  '{:tine/guide-announced? true\n :default-queries {:journals [{:title "ToDos" :query (task TODO DOING)}]}}\n');
const FILLER = Array.from({ length: 30 }, (_, i) => `- today filler ${i}`);
fs.writeFileSync(path.join(graph, 'journals', `${stem(date(0))}.md`), [
  ...FILLER,
  '- FOLD today parent', '  - child one', '  - child two', '  - child three', '  - child four',
  `- TODO TICK today agenda\n  SCHEDULED: ${org(date(0))}`,
  '- TODO TICK today plain',
  '- today last 0', '- today last 1', '- today last 2',
].join('\n') + '\n');
for (let ago = 1; ago <= 8; ago++) {
  const lines = Array.from({ length: 10 }, (_, i) => `- day ${ago} row ${i}\n  - child a\n  - child b`);
  if (ago === 1) lines.splice(3, 0, '- FOLD older parent', '  - older child one', '  - older child two', '  - older child three',
    `- TODO TICK older agenda\n  SCHEDULED: ${org(date(-1))}`, '- TODO TICK older plain');
  fs.writeFileSync(path.join(graph, 'journals', `${stem(date(-ago))}.md`), lines.join('\n') + '\n');
}
const tasks = [];
for (let offset = -2; offset <= 5; offset++) {
  tasks.push(`- TODO scheduled ${offset}\n  SCHEDULED: ${org(date(offset))}`);
  if (offset >= 0) tasks.push(`- TODO deadline ${offset}\n  DEADLINE: ${org(date(offset))}`);
}
fs.writeFileSync(path.join(graph, 'pages/Tasks.md'), tasks.join('\n') + '\n');
// Every older day is referenced, so each day carries a Linked References section like a real graph.
fs.writeFileSync(path.join(graph, 'pages/Refs.md'),
  Array.from({ length: 8 }, (_, i) => `- note about [[${title(date(-(i + 1)))}]]`).join('\n') + '\n');
const homeEnv = process.platform === 'win32' ? {} : { HOME: path.join(run, 'home'), USERPROFILE: path.join(run, 'home') };
const env = { ...process.env, TINE_GRAPH: graph, ...homeEnv,
  XDG_DATA_HOME: path.join(run, 'data'), XDG_CONFIG_HOME: path.join(run, 'config'),
  XDG_CACHE_HOME: path.join(run, 'cache'), XDG_STATE_HOME: path.join(run, 'state'),
  APPDATA: path.join(run, 'appdata'), LOCALAPPDATA: path.join(run, 'localappdata'), TMPDIR: path.join(run, 'app-tmp'),
  WEBKIT_DISABLE_DMABUF_RENDERER: '1', WEBKIT_DISABLE_COMPOSITING_MODE: '1', LIBGL_ALWAYS_SOFTWARE: '1', GDK_BACKEND: 'x11' };
const owner = accountHomeFixture(run, run, env);
Object.assign(env, owner.env);
const port = Number(process.env.E2E_DRIVER_PORT || 4946), nativePort = Number(process.env.E2E_NATIVE_PORT || 4947);

// route: how the action is delivered. settle: observation window after it.
const FOLD = (day, route, text) => ({ day, route, text, kind: 'fold' });
const TICK = (day, route, text) => ({ day, route, text, kind: 'tick' });
const PHASES = [
  ...['today', 'older'].flatMap(day => ['click', 'key'].flatMap(route => {
    const text = `FOLD ${day} parent`;
    return [{ name: `fold-${day}-${route}`, ...FOLD(day, route, text), collapsed: true },
      { name: `unfold-${day}-${route}`, ...FOLD(day, route, text), collapsed: false }];
  })),
  ...['today', 'older'].flatMap(day => ['agenda', 'plain'].flatMap(member => ['click', 'key'].flatMap(route => {
    const text = `TICK ${day} ${member}`;
    return [{ name: `tick-${day}-${member}-${route}`, ...TICK(day, route, text), done: true },
      { name: `untick-${day}-${member}-${route}`, ...TICK(day, route, text), done: false }];
  }))),
];
const only = process.env.E2E_PHASES ? new RegExp(process.env.E2E_PHASES) : null;
const SETTLE_MS = Number(process.env.E2E_SETTLE_MS || 1600);
const results = { commit: process.env.TINE_SOURCE_REVISION || 'working-tree', app, platform: process.platform, graph, manifest,
  phases: [], captureFailures: [] };
let browser, target, driver, log;
let stage = 'startup';

// The page's own row for a block, not its copy in the agenda or a references section. Each callback runs in the
// page, so the lookup is repeated inline rather than shared.
const center = id => browser.execute(id => {
  const scroller = document.querySelector('.main-content');
  const el = [...scroller.querySelectorAll(`[data-block-id="${id}"]`)].find(e => !e.closest('.agenda-block, .journal-linked-references, .query-block'));
  const s = scroller.getBoundingClientRect(), b = el.getBoundingClientRect();
  scroller.scrollTop += (b.top - s.top) - (s.height / 2 - 40);
}, id);
const blockId = text => browser.execute(text => {
  const hit = [...document.querySelectorAll('.main-content .ls-block')]
    .filter(el => !el.closest('.agenda-block, .journal-linked-references, .query-block'))
    .find(el => el.querySelector('.block-content')?.textContent.replace(/\s+/g, ' ').includes(text));
  return hit?.dataset.blockId ?? null;
}, text);
const state = id => browser.execute(id => {
  const el = [...document.querySelectorAll(`.main-content [data-block-id="${id}"]`)].find(e => !e.closest('.agenda-block, .journal-linked-references, .query-block'));
  return { children: el?.querySelectorAll('.block-children .ls-block').length ?? 0,
    done: !!el?.querySelector('.block-task-checkbox.checked'), marker: el?.querySelector('.block-marker')?.textContent ?? null };
}, id);
const clickEl = (id, selector) => browser.execute((id, selector) => {
  const own = [...document.querySelectorAll(`.main-content [data-block-id="${id}"]`)].find(e => !e.closest('.agenda-block, .journal-linked-references, .query-block'));
  const box = own.querySelector(selector).getBoundingClientRect();
  return { x: Math.round(box.left + box.width / 2), y: Math.round(box.top + box.height / 2) };
}, id, selector);
async function pointerClick({ x, y }) {
  await browser.performActions([{ type: 'pointer', id: 'gh944-fold-mouse', parameters: { pointerType: 'mouse' }, actions: [
    { type: 'pointerMove', origin: 'viewport', duration: 0, x, y }, { type: 'pointerDown', button: 0 }, { type: 'pointerUp', button: 0 },
  ] }]);
  await browser.releaseActions();
}
async function textPoint(id) {
  // Click the title text itself so the editor opens with a caret in this block.
  return browser.execute(id => {
    const own = [...document.querySelectorAll(`.main-content [data-block-id="${id}"]`)].find(e => !e.closest('.agenda-block, .journal-linked-references, .query-block'));
    const content = own.querySelector('.block-content');
    const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const range = document.createRange();
      range.selectNodeContents(walker.currentNode);
      const box = range.getBoundingClientRect();
      if (box.width > 0) return { x: Math.round(box.left + Math.min(box.width - 1, 12)), y: Math.round(box.top + box.height / 2) };
    }
    throw new Error('no text to click');
  }, id);
}
async function deliver(phase, id) {
  const mod = process.platform === 'darwin' ? 'Meta' : 'Control';
  if (phase.kind === 'fold') {
    if (phase.route === 'click') return pointerClick(await clickEl(id, '.collapse-toggle'));
    await pointerClick(await textPoint(id));
    await browser.waitUntil(() => browser.execute(() => document.activeElement?.matches('textarea.block-editor, .cm-content')), { timeout: 10_000 });
    await browser.keys([mod, phase.collapsed ? 'ArrowUp' : 'ArrowDown']);
    return;
  }
  if (phase.route === 'click') return pointerClick(await clickEl(id, '.block-task-checkbox'));
  await pointerClick(await textPoint(id));
  await browser.waitUntil(() => browser.execute(() => document.activeElement?.matches('textarea.block-editor, .cm-content')), { timeout: 10_000 });
  // TODO -> DOING -> DONE under the default workflow: two cycles tick the task (the agenda drops it).
  await browser.keys([mod, 'Enter']);
  if (phase.done) { await sleep(150); await browser.keys([mod, 'Enter']); }
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
  try { await browser.setWindowRect(0, 0, 1280, 800); } catch (error) { results.windowRectError = String(error); }
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector('.nav-item')), { timeout: 60_000 });
  await openJournals(browser);
  // The built-in list starts collapsed on a busy graph; the reporter's was open.
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector('.agenda-block .query-collapse')), { timeout: 30_000 });
  await browser.execute(() => { const toggle = document.querySelector('.agenda-block .query-collapse'); if (toggle.classList.contains('collapsed')) toggle.click(); });
  // Rows mount only near the viewport, and the reporter-like ToDos list pushes the agenda far down.
  await browser.waitUntil(() => browser.execute(() => { document.querySelector('.agenda-block')?.scrollIntoView(); return true; }), { timeout: 30_000 });
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll('.agenda-block .ls-block')]
    .some(row => row.textContent.includes('TICK today agenda'))), { timeout: 30_000, timeoutMsg: 'agenda never listed the scheduled task' });
  await assertAppLease(owner);
  results.environment = await browser.execute(() => ({ userAgent: navigator.userAgent, dpr: devicePixelRatio,
    width: innerWidth, height: innerHeight, date: new Date().toString(), url: location.href }));
  for (const phase of PHASES) {
    if (only && !only.test(phase.name)) continue;
    stage = phase.name;
    const id = await blockId(phase.text);
    if (!id) throw new Error(`block ${phase.text} not found`);
    // Scroll so the target is mid-screen with the agenda above it; let lazy sections mount; recentre.
    for (let i = 0; i < 2; i++) { await center(id); await sleep(700); }
    const before = await state(id);
    const wantState = phase.kind === 'fold' ? before.children > 0 === phase.collapsed : before.done !== phase.done;
    if (!wantState) { results.phases.push({ phase: phase.name, skipped: `already in target state ${JSON.stringify(before)}` }); continue; }
    const init = await browser.execute(installFoldProbe, id);
    await sleep(120);
    const t0 = Date.now();
    await deliver(phase, id);
    // A keyboard action ends when the user leaves the block; saves and query refreshes follow that.
    if (phase.route === 'key') { await sleep(700); await browser.keys('Escape'); }
    await sleep(SETTLE_MS);
    const recorded = await browser.execute(() => window.__gh944fold.stop());
    const summary = summarizeFold(recorded);
    const classification = classifyFold(summary);
    const after = await state(id);
    results.phases.push({ phase: phase.name, day: phase.day, route: phase.route, kind: phase.kind, init, before, after,
      classification, summary, queries: recorded.queries, actionStartedMsAfterInstall: t0, frames: recorded.frames });
    console.log(`GH944 FOLD ${phase.name}: ${classification} maxExcursion=${summary.maxExcursion?.toFixed(1)} final=${summary.finalDisplacement?.toFixed(1)} `
      + `scrollTop=${summary.scrollTopDelta.toFixed(1)} stillScrolls=${summary.stillScrolls} agenda=[${summary.agendaHeightRange}] `
      + `queries=${JSON.stringify(summary.queryCommands)} ${JSON.stringify((recorded.queries ?? []).map(q => q.body.slice(0, 160)))} sections=${JSON.stringify(summary.sectionHeightChanges)}`);
    await browser.keys('Escape');
    fs.writeFileSync(path.join(out, 'gh944-fold-probe.json'), JSON.stringify(results, null, 2));
  }
  results.semanticPassed = true;
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, 'failure.png')); } catch {}
  try { results.failureState = await browser?.execute(() => ({ agenda: document.querySelector('.agenda-block')?.textContent.slice(0, 600),
    rows: document.querySelectorAll('.agenda-block .ls-block').length, queries: [...document.querySelectorAll('.default-journal-query')].map(q => q.textContent.slice(0, 120)),
    sections: [...document.querySelectorAll('.today-sections, .today-queries')].map(q => q.className) })); } catch {}
  results.failure = { stage, observed: String(error) };
  throw error;
} finally {
  fs.writeFileSync(path.join(out, 'gh944-fold-probe.json'), JSON.stringify(results, null, 2));
  try { await browser?.deleteSession(); } catch {}
  if (driver) {
    if (process.platform === 'win32') spawnSync('taskkill', ['/PID', String(driver.pid), '/T', '/F'], { stdio: 'ignore' });
    else try { process.kill(-driver.pid, 'SIGKILL'); } catch {}
  }
  stopWebdriverApplication(target);
  if (log !== undefined) fs.closeSync(log);
  owner.close?.();
}
