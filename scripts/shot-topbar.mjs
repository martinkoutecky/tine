import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
const PORT = 5197;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function wait(u,t=60){for(let i=0;i<t;i++){try{if((await fetch(u)).ok)return}catch{}await sleep(250)}throw new Error("no server")}
try {
  await wait(`http://localhost:${PORT}/`);
  const b = await chromium.launch();
  const p = await b.newPage({ viewport: { width: 900, height: 300 } });
  await p.goto(`http://localhost:${PORT}/`);
  await p.waitForSelector(".ls-block", { timeout: 5000 });
  // open a 2nd tab by middle-clicking a page link
  await p.locator(".page-ref").first().click({ button: "middle" });
  await sleep(400);
  await p.screenshot({ path: "screenshots/topbar.png", clip: { x: 0, y: 0, width: 900, height: 90 } });
  await b.close(); server.kill("SIGKILL"); console.log("ok"); process.exit(0);
} catch (e) { console.error(String(e)); server.kill("SIGKILL"); process.exit(1); }
