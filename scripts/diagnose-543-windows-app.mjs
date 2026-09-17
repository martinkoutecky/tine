import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';
import { remote } from 'webdriverio';
import { freeLoopbackPort, startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, selectWebdriverWindowWithSelector } from './e2e-capabilities.mjs';

const app = process.env.TINE_APP;
if (process.platform !== 'win32' || !app) throw new Error('Windows and TINE_APP required');
const artifacts = path.resolve('test-results/diagnose-543-app');
fs.mkdirSync(artifacts, { recursive: true });
const records = [];
for (const count of [1000, 10000]) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tine-543-app-'));
  const graph = path.join(root, 'graph');
  for (const dir of ['pages', 'journals', 'logseq']) fs.mkdirSync(path.join(graph, dir), { recursive: true });
  for (let page = 0; page < count; page++) {
    let text = `title:: Topic ${page} 你好\n\n`;
    for (let block = 0; block < 60; block++) text += `- outline sentinel543 你好世界 page ${page} block ${block} [[Topic ${(page + 1) % count} 你好]] #tag${block % 10}\n`;
    fs.writeFileSync(path.join(graph, 'pages', `主题-${String(page).padStart(5, '0')}.md`), text);
  }
  const now = new Date();
  const stem = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, '0')}_${String(now.getDate()).padStart(2, '0')}`;
  fs.writeFileSync(path.join(graph, 'journals', `${stem}.md`), '- DIAG543 launch marker\n');
  for (const phase of ['cold', 'reopen']) {
    const prefix = path.join(artifacts, `${count}-${phase}`);
    const env = { ...process.env, TINE_GRAPH: graph, TINE_DEBUG: '1', TINE_DEBUG_LOG: `${prefix}-debug.log`,
      APPDATA: path.join(root, 'appdata'), LOCALAPPDATA: path.join(root, 'localappdata'),
      E2E_WEBVIEW_USER_DATA_ROOT: path.join(root, 'webview'),
      TINE_E2E_APPLICATION_STDOUT_LOG: `${prefix}-stdout.log`, TINE_E2E_APPLICATION_STDERR_LOG: `${prefix}-stderr.log` };
    const nativePort = await freeLoopbackPort();
    const driverPort = await freeLoopbackPort(new Set([nativePort]));
    const started = Date.now();
    const target = await startWebdriverApplication(app, env, nativePort);
    const driverLog = fs.openSync(`${prefix}-driver.log`, 'w');
    const driver = spawn('msedgedriver', [`--port=${driverPort}`], { env: target.env, stdio: ['ignore', driverLog, driverLog] });
    let browser;
    try {
      await sleep(1500);
      browser = await remote({ hostname: '127.0.0.1', port: driverPort, path: '/',
        capabilities: tauriCapabilities(app, 'default', process.platform, target.debuggerAddress),
        logLevel: 'error', connectionRetryCount: 1, connectionRetryTimeout: 60000 });
      await selectWebdriverWindowWithSelector(browser, 'button[title^="Search (Ctrl+K)"]', 90000);
      await browser.execute(() => {
        const native = window.__TAURI_INTERNALS__;
        const original = native.invoke.bind(native);
        window.__diag543 = [];
        native.invoke = async (command, args, options) => {
          const record = { command, start: performance.now() };
          const watched = /search|query|backlinks|warm_done/.test(command);
          if (watched) window.__diag543.push(record);
          try { const value = await original(command, args, options); record.ok = true; return value; }
          catch (error) { record.error = error; throw error; }
          finally { record.ms = performance.now() - record.start; }
        };
      });
      await browser.$('button[title^="Search (Ctrl+K)"]').click();
      await browser.$('.switcher-input').waitForExist({ timeout: 10000 });
      await browser.$('.switcher-input').setValue('sentinel543');
      let passed = false;
      const deadline = Date.now() + (count === 1000 ? 180000 : 600000);
      while (Date.now() < deadline) {
        const snapshot = await browser.execute(() => ({
          status: document.querySelector('.switcher [role="status"]')?.textContent ?? null,
          error: document.querySelector('.switcher [role="alert"]')?.textContent ?? null,
          matches: [...document.querySelectorAll('.switcher-row.block-result')].map(e => e.textContent),
          ipc: window.__diag543.slice(-12),
        }));
        const record = { count, phase, elapsedMs: Date.now() - started, ...snapshot };
        records.push(record);
        console.log('APP543', JSON.stringify(record));
        fs.writeFileSync(path.join(artifacts, 'progress.json'), JSON.stringify(records, null, 2));
        if (snapshot.matches.some(text => text.includes('sentinel543')) && !snapshot.status && !snapshot.error) { passed = true; break; }
        await sleep(2000);
      }
      await browser.saveScreenshot(`${prefix}.png`);
      fs.writeFileSync(`${prefix}-ipc.json`, JSON.stringify(await browser.execute(() => window.__diag543), null, 2));
      if (!passed) throw new Error(`APP543 timeout pages=${count} phase=${phase}`);
      console.log(`APP543 RESULT pages=${count} phase=${phase} elapsedMs=${Date.now() - started} PASS`);
    } finally {
      try { await browser?.saveScreenshot(`${prefix}-final.png`); } catch {}
      try { await browser?.deleteSession(); } catch {}
      spawnSync('taskkill', ['/PID', String(driver.pid), '/T', '/F'], { stdio: 'ignore' });
      stopWebdriverApplication(target);
      fs.closeSync(driverLog);
    }
  }
}
