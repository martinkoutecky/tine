// External native-input/screenshot driver. Never uses private graph or app-data.
import { spawn } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { mkdir, cp, writeFile, readFile, access } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const out = path.resolve(process.env.TINE_SPIKE_MW || path.join(root, 'test-results/spike-multiwindow'));
const graph = path.join(out, 'graph');
await mkdir(out, { recursive: true });
await cp(path.join(root, 'scripts/spike-multiwindow/fixture'), graph, { recursive: true });
for (const dir of ['data', 'config', 'cache', 'state']) await mkdir(path.join(out, dir), { recursive: true });
const env = { ...process.env, TINE_SPIKE_MW: out, TINE_SPIKE_GRAPH: graph,
  TINE_SPIKE_MW_OSKEYS: '1', TINE_GPU: '0', XDG_DATA_HOME: path.join(out, 'data'),
  XDG_CONFIG_HOME: path.join(out, 'config'), XDG_CACHE_HOME: path.join(out, 'cache'), XDG_STATE_HOME: path.join(out, 'state') };
const app = path.resolve(process.env.TINE_APP || path.join(root, 'target/debug', process.platform === 'win32' ? 'tine.exe' : 'tine'));
const errors = [];
const stderrFile = createWriteStream(path.join(out, 'app.stderr'));
const stdoutFile = createWriteStream(path.join(out, 'app.stdout'));
let stderr = '', stdout = '', ended = false, exitCode = null, signal = null;
const child = spawn(app, [], { cwd: root, env });
child.stderr.on('data', b => { stderr += b; stderrFile.write(b); });
child.stdout.on('data', b => { stdout += b; stdoutFile.write(b); });
child.on('error', e => { errors.push(String(e)); ended = true; });
child.on('exit', (code, sig) => { ended = true; exitCode = code; signal = sig; });
const pause = ms => new Promise(r => setTimeout(r, ms));
const exists = async name => access(path.join(out, name)).then(() => true, () => false);
async function external(command, args) {
  return new Promise((resolve, reject) => {
    const p = spawn(command, args, { env }); let output = '';
    const deadline = setTimeout(() => { p.kill(); reject(new Error(`${command} timed out after 15s`)); }, 15000);
    p.stdout.on('data', b => { output += b; }); p.stderr.on('data', b => { output += b; });
    p.on('error', error => { clearTimeout(deadline); reject(error); });
    p.on('exit', code => { clearTimeout(deadline); code === 0 ? resolve(output.trim()) : reject(new Error(`${command} exit ${code}: ${output}`)); });
  });
}
async function screenshot() {
    try {
      if (process.platform === 'linux') {
        await external('import', ['-window', 'root', path.join(out, 'desktop.png')]);
        const titles = JSON.parse(await readFile(path.join(out, 'screenshots.ready'), 'utf8').catch(() => '[]'));
        if (!titles.length) return;
        for (const [name, title] of [['main', titles.find(w => w.label === 'main').title], ['popup', 'Tine Spike Popup']]) {
          const ids = await external('xdotool', ['search', '--onlyvisible', '--name', `^${title.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`]);
          await external('import', ['-window', ids.split('\n')[0], path.join(out, `${name}.png`)]);
        }
      } else if (process.platform === 'win32') {
        await external('powershell', ['-NoProfile', '-Command', `Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $r=[System.Windows.Forms.SystemInformation]::VirtualScreen; $b=New-Object System.Drawing.Bitmap($r.Width,$r.Height); $g=[System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($r.Left,$r.Top,0,0,$r.Size); $b.Save('${path.join(out, 'desktop.png').replaceAll("'", "''")}'); $g.Dispose(); $b.Dispose()`]);
        const titles = JSON.parse(await readFile(path.join(out, 'screenshots.ready'), 'utf8').catch(() => '[]'));
        for (const [name, title] of [['main', titles.find(w => w.label === 'main')?.title], ['popup', 'Tine Spike Popup']]) {
          if (!title) continue;
          await external('powershell', ['-NoProfile', '-Command', `Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $w=New-Object -ComObject WScript.Shell; if (-not $w.AppActivate('${title.replaceAll("'", "''")}')) { throw 'window focus failed' }; Start-Sleep -Milliseconds 500; $r=[System.Windows.Forms.SystemInformation]::VirtualScreen; $b=New-Object System.Drawing.Bitmap($r.Width,$r.Height); $g=[System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($r.Left,$r.Top,0,0,$r.Size); $b.Save('${path.join(out, `${name}.png`).replaceAll("'", "''")}'); $g.Dispose(); $b.Dispose()`]);
        }
      } else {
        await external('screencapture', ['-x', path.join(out, 'desktop.png')]);
        const info = await external('swift', ['-e', `import CoreGraphics; import Foundation
let windows = CGWindowListCopyWindowInfo(.optionAll, kCGNullWindowID) as! [[String: Any]]
let own = windows.filter { ($0[kCGWindowOwnerPID as String] as? Int) == ${child.pid} && ($0[kCGWindowLayer as String] as? Int) == 0 }
let data = try! JSONSerialization.data(withJSONObject: own)
print(String(data: data, encoding: .utf8)!)`]);
        await writeFile(path.join(out, 'native-windows.json'), info);
        for (const window of JSON.parse(info)) {
          const name = window.kCGWindowName === 'Tine Spike Popup' ? 'popup' : 'main';
          if (window.kCGWindowName === 'Quick Capture') continue;
          await external('screencapture', ['-x', '-l', String(window.kCGWindowNumber), path.join(out, `${name}.png`)]);
        }
      }
      await writeFile(path.join(out, 'screenshot.json'), JSON.stringify({ status: 'pass', detail: 'External OS capture; desktop includes both windows. Inspect image for actual decorations and parity.' }, null, 2));
    } catch (e) {
      await writeFile(path.join(out, 'screenshot.json'), JSON.stringify({ status: 'error', detail: String(e) }, null, 2));
    }
}
let shot = false, fallbackShot = false, keys = false;
const started = Date.now();
while (!ended && Date.now() - started < 120000) {
  if (!shot && await exists('screenshots.ready')) {
    shot = true;
    await screenshot();
  }
  if (!keys && await exists('ready.ready')) {
    keys = true;
    try {
      if (process.platform === 'linux') {
        const id = (await external('xdotool', ['search', '--onlyvisible', '--name', '^Tine Spike Popup$'])).split('\n')[0];
        await external('xdotool', ['windowactivate', '--sync', id]);
        await external('xdotool', ['type', '--clearmodifiers', '--delay', '80', 'spike123']);
        await external('xdotool', ['key', '--clearmodifiers', 'Return']);
      } else if (process.platform === 'win32') {
        await external('powershell', ['-NoProfile', '-Command', `Add-Type -AssemblyName System.Windows.Forms; $w=New-Object -ComObject WScript.Shell; if (-not $w.AppActivate('Tine Spike Popup')) { throw 'popup focus failed' }; Start-Sleep -Milliseconds 500; [System.Windows.Forms.SendKeys]::SendWait('spike123{ENTER}')`]);
      } else {
        await external('osascript', ['-e', 'tell application "System Events" to keystroke "spike123"', '-e', 'tell application "System Events" to key code 36']);
      }
      await writeFile(path.join(out, 'oskeys.json'), JSON.stringify({ status: 'pass', detail: 'Native OS keyboard command completed; R in result.json checks DOM and disk.' }, null, 2));
    } catch (e) {
      await writeFile(path.join(out, 'oskeys.json'), JSON.stringify({ status: 'error', detail: String(e) }, null, 2));
    }
  }
  if (!shot && !fallbackShot && Date.now() - started > 20000) { fallbackShot = true; await screenshot(); }
  await pause(100);
}
if (!shot) await screenshot();
if (!ended) { errors.push('App hung: 120s deadline'); child.kill('SIGKILL'); await pause(500); }
await Promise.all([new Promise(r => stderrFile.end(r)), new Promise(r => stdoutFile.end(r))]);
await writeFile(path.join(out, 'app.stderr'), stderr);
await writeFile(path.join(out, 'app.stdout'), stdout);
await writeFile(path.join(out, 'process.json'), JSON.stringify({ exitCode, signal, errors }, null, 2));
let result;
try { result = JSON.parse(await readFile(path.join(out, 'result.json'), 'utf8')); }
catch { errors.push('No result.json: crash, failed launch, or hang'); }
if (result) {
  for (const id of ['C1','C2','C3','C4','C5','C6','C7','C8','C9','R']) result[id] ??= { status: 'fail', detail: 'App did not complete this check; see app.stderr and process.json' };
  try {
    const capture = JSON.parse(await readFile(path.join(out, 'screenshot.json'), 'utf8'));
    result.C9 ??= capture;
    result.C9.externalCapture = capture;
    if (capture.status !== 'pass') result.C9.status = capture.status;
  }
  catch { result.C9 = { status: 'error', detail: 'No external screenshots produced' }; }
  await writeFile(path.join(out, 'result.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
}
// Checks may fail (app exit 1) without failing CI. Native crashes/hangs do fail.
if (errors.length || signal || ![0, 1].includes(exitCode)) { console.error(errors, { exitCode, signal }); process.exitCode = 1; }
