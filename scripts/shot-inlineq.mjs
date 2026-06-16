import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
const PORT = 5198;
const server = spawn("npx", ["vite","preview","--port",String(PORT),"--strictPort"], { stdio: "ignore" });
async function wait(u,t=60){for(let i=0;i<t;i++){try{if((await fetch(u)).ok)return}catch{}await sleep(250)}throw new Error("no server")}
try {
  await wait(`http://localhost:${PORT}/`);
  const b = await chromium.launch();
  const p = await b.newPage({ viewport: { width: 860, height: 1500 } });
  const errs=[]; p.on("console",m=>m.type()==="error"&&errs.push(m.text())); p.on("pageerror",e=>errs.push(String(e)));
  await p.goto(`http://localhost:${PORT}/`);
  await p.waitForSelector(".ls-block", { timeout: 5000 });
  await sleep(800);
  await p.screenshot({ path: "screenshots/inlineq.png", fullPage: true });
  console.log(errs.length ? "ERRORS:\n"+errs.join("\n") : "no console errors");
  await b.close(); server.kill("SIGKILL"); process.exit(0);
} catch(e){ console.error(String(e)); server.kill("SIGKILL"); process.exit(1); }
