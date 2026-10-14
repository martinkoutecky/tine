// Real engine + today's journal renderer, with both OG 0.10.9 defaults and
// the older reference-based NEXT saved in the GH #690 report.
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openJournals, openPageByName } from "./lib/e2e-navigation.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/gh690");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const date = offset => { const d = new Date(); d.setDate(d.getDate() + offset); return d; };
const iso = offset => { const d = date(offset); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`; };
const file = offset => `${iso(offset).replaceAll("-", "_")}.md`;
const query = (title, relation, inputs, markers, bounds, transform = "") => `{:title "${title}"
 :query [:find (pull ?h [*]) :in $ ?start ?next :where
 [?h :block/marker ?marker] [(contains? #{${markers}} ?marker)]
 [?h ${relation} ?p] [?p :block/journal? true] [?p :block/journal-day ?d]
 [(${bounds[0]} ?d ?start)] [(${bounds[1]} ?d ?next)]]
 :inputs [${inputs}] ${transform} :collapsed? false}`;
const now = query("🔨 NOW", ":block/page", ":14d :today", '"NOW" "DOING"', [">=", "<="], ':result-transform (fn [result] (sort-by (fn [h] (get h :block/priority "Z")) result)) :group-by-page? false').replaceAll("?next", "?today");
const next = query("📅 NEXT", ":block/page", ":today :7d-after", '"NOW" "LATER" "TODO"', [">", "<"], ":group-by-page? false");
const refs = query("Older NEXT", ":block/ref-pages", ":today :14d-after", '"NOW" "LATER" "TODO"', [">", "<"]);
const config = `{:journal/page-title-format "yyyy-MM-dd" :feature/disable-scheduled-and-deadline-query? true
 :default-queries {:journals [${now} ${next} ${refs}
 {:title "Unsupported config" :query [:find ?h :where (unknown ?h)]}]}}`;
fs.writeFileSync(path.join(graph, "logseq/config.edn"), config);
fs.writeFileSync(path.join(graph, "journals", file(0)), "- NOW no-priority\n- DOING [#A] first-priority\n- NOW [#B] second-priority\n");
fs.writeFileSync(path.join(graph, "journals", file(3)), "- TODO future-journal-task\n");
fs.writeFileSync(path.join(graph, "journals", file(7)), "- TODO journal-end-boundary\n");
fs.writeFileSync(path.join(graph, "pages/Tasks.md"), `- TODO referenced-inside [[${iso(3)}]]
- TODO referenced-start [[${iso(0)}]]
- TODO referenced-end [[${iso(14)}]]
- TODO split-dates [[${iso(-1)}]] [[${iso(15)}]]
- Parent [[${iso(3)}]]
  - TODO inherited-reference
`);
const base = execFileSync("rtk", ["proxy", "git", "rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const env = { ...process.env, TINE_GRAPH: graph,
 XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
 WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4790);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"),
 ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
 { env, stdio: ["ignore", log, log], detached: true });
let browser;
let stage = "startup";
async function observe() {
 await browser.waitUntil(() => browser.execute(() => {
   const regions = [...document.querySelectorAll(".default-journal-query")];
   return regions.length === 4 && regions[2].textContent.includes("referenced-inside");
 }), { timeout: 30_000 });
 const state = await browser.execute(() => [...document.querySelectorAll(".default-journal-query")].map(region => ({
   text: region.textContent,
   rows: [...region.querySelectorAll(".query-group .ls-block")].map(block => block.textContent),
   collapsed: !!region.querySelector(".query-collapse.collapsed"),
   detailsClosed: !region.querySelector("details")?.open,
 })));
 const require = (condition, reason) => { if (!condition) throw new Error(`${reason}: ${JSON.stringify(state)}`); };
 require(state[0].rows.length === 3 && state[0].rows[0].includes("first-priority") && state[0].rows[1].includes("second-priority") && state[0].rows[2].includes("no-priority"), "NOW priority order");
 require(state[1].rows.length === 1 && state[1].rows[0].includes("future-journal-task"), "0.10.9 NEXT own-page range");
 require(state[2].rows.length === 1 && state[2].rows[0].includes("referenced-inside"), "older NEXT direct refs and exclusive bounds");
 require(state.slice(0, 3).every(region => !region.collapsed), "collapsed false renders results");
 require(state[3].text.includes("Unsupported config") && state[3].text.includes("Tine can't run this query from config.edn yet") && state[3].detailsClosed, "plain refusal with closed details");
 return state;
}
try {
 browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
   capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/debug/tine") } } });
 const initial = await observe();
 stage = "expand technical details";
 await browser.$(".query-unsupported summary").click();
 if (!await browser.execute(() => {
   const details = document.querySelector(".query-unsupported details");
   return details?.open && details.textContent.includes("unknown");
 })) throw new Error("Technical details did not open with the unsupported clause");
 await browser.$(".query-unsupported summary").click();
 stage = "navigate away and return";
 await openPageByName(browser, "Tasks");
 await openJournals(browser);
 await observe();
 stage = "external task edit";
 fs.appendFileSync(path.join(graph, "pages/Tasks.md"), `- LATER live-reference [[${iso(4)}]]\n`);
 await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".default-journal-query")[2]?.textContent.includes("live-reference")), { timeout: 30_000 });
 await browser.saveScreenshot(path.join(run, "passed.png"));
 fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify({ base, workingDiff: true, initial, liveEdit: "passed" }, null, 2));
 console.log(`PASS gh690: stock queries, older NEXT, refusal, return, live edit (${run})`);
} catch (error) {
 try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
 try { fs.writeFileSync(path.join(run, "failure.html"), await browser.getPageSource()); } catch {}
 fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${base} + diff\nJourney: ${stage}\nExpected: NOW priority order, NEXT ranges, visible plain refusal, live update\nObserved: ${error.stack}\nClassification: ambiguous; see DOM and driver log\n`);
 throw error;
} finally {
 try { await browser?.deleteSession(); } catch {}
 try { process.kill(-driver.pid, "SIGKILL"); } catch {}
 fs.closeSync(log);
}
