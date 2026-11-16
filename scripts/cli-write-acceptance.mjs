// CLI spec §3: fixed 50-operation acceptance and warm-owner per-verb budget.
// All output is counts/timings; graph names and content never enter artifacts.
// Usage: node scripts/cli-write-acceptance.mjs /absolute/path/to/tine SOURCE OUTPUT
// SOURCE is read only. OUTPUT must not exist and contains the disposable copy.
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { performance } from "node:perf_hooks";
import assert from "node:assert/strict";

const [app, source, output] = process.argv.slice(2);
if (!app || !source || !output || fs.existsSync(output)) throw new Error("supply app, source and a new disposable output directory");
assert(fs.lstatSync(source).isDirectory(),"source must be a real directory, not a symlink");
const canonicalSource = fs.realpathSync(source);
const canonicalOutput = path.join(fs.realpathSync(path.dirname(path.resolve(output))),path.basename(output));
assert(canonicalOutput !== canonicalSource && !canonicalOutput.startsWith(canonicalSource + path.sep),"output must be outside the read-only source");
// Refuse linked input: a copied symlink could still point back at the real graph.
function rejectLinks(dir) {
  for (const item of fs.readdirSync(dir,{withFileTypes:true})) {
    assert(!item.isSymbolicLink(),"acceptance source must contain no symlinks");
    if (item.isDirectory()) rejectLinks(path.join(dir,item.name));
  }
}
rejectLinks(source);
fs.mkdirSync(output, { recursive: true });
const graph = path.join(output, "graph");
fs.cpSync(source, graph, { recursive: true, dereference: false });
const env = { ...process.env, XDG_DATA_HOME: path.join(output, "data"), XDG_CONFIG_HOME: path.join(output, "config"), XDG_CACHE_HOME: path.join(output, "cache") };
const timings = Object.fromEntries(["add", "edit", "prop", "move", "delete"].map(v => [v, []]));
function call(args, measure) {
  const start = performance.now();
  const result = spawnSync(app, [...args, "--graph", graph, "--json"], { env, encoding: "utf8", timeout: 60_000 });
  if (measure) timings[measure].push(performance.now() - start);
  // Do not leak real graph content in an error report.
  assert.equal(result.status, 0, `CLI ${args[0]} exit ${result.status}`);
  return JSON.parse(result.stdout);
}
function files(dir, prefix = "") {
  const found = new Map();
  for (const item of fs.readdirSync(dir, { withFileTypes: true })) {
    const relative = path.join(prefix, item.name);
    if (item.isSymbolicLink()) continue;
    if (item.isDirectory()) for (const [k,v] of files(path.join(dir,item.name),relative)) found.set(k,v);
    else found.set(relative, fs.readFileSync(path.join(dir,item.name)));
  }
  return found;
}
function doctor() {
  const result = spawnSync(app, ["doctor", graph], { env, encoding: "utf8", timeout: 60_000 });
  // Keep only a status here: doctor may name real pages.
  return { exit: result.status };
}
const baseline = files(graph);
const report = { operations: 0, doctorBefore: doctor() };
try {
  const pages = call(["pages"]).pages;
  const targets = [];
  for (const prefix of ["pages/", "journals/"]) {
    const candidate = pages.find(p => p.id?.startsWith(prefix) && p.id.endsWith(".md") && (prefix !== "journals/" || /^journals\/\d{4}_(0[1-9]|1[0-2])_(0[1-9]|1\d|2[0-8])\.md$/.test(p.id)));
    if (candidate) targets.push(candidate);
  }
  const org = pages.find(p => p.id?.endsWith(".org"));
  if (org) targets.push(org);
  assert(targets.some(p => p.id.startsWith("pages/")), "a named page is required");
  assert(targets.some(p => p.id.startsWith("journals/")), "a journal is required");
  // The first round creates a page through add's guarded CreateNew path.
  const created = { name: "Tine CLI synthetic acceptance 20261017", id: "pages/Tine CLI synthetic acceptance 20261017.md" };
  assert(!pages.some(p => p.name === created.name));
  const cycle = (target, index, measure) => {
    const org = target.id.endsWith(".org");
    const prefix = `CLI proof ${index}`;
    const place = target.id.startsWith("journals/") ? [`--journal=${path.basename(target.id,".md").replaceAll("_","-")}`] : ["--page",target.name];
    const added = call(["add", ...place, "--author", "acceptance", org ? `* ${prefix}\n** nested\n* ${prefix} sibling` : `- ${prefix}\n\t- nested\n- ${prefix} sibling`], measure && "add");
    const root = added.blocks[0];
    const sibling = added.blocks[2];
    assert(root && sibling, "the forest returns all three created handles");
    const edited = call(["edit", root, `${prefix} edited`], measure && "edit");
    const prop = call(["prop", "set", edited.blocks[0], "proof", "yes"], measure && "prop");
    call(["move", sibling, "--under", prop.blocks[0]], measure && "move");
    call(["delete", prop.blocks[0]], measure && "delete");
  };
  for (let i = 0; i < 10; i++) {
    cycle(i === 0 ? created : targets[(i-1) % targets.length], i, false);
    report.operations += 5;
  }
  const after = files(graph);
  // Every pre-existing file must be identical, including all untouched blocks.
  const changed = [...baseline].filter(([name, bytes]) => !after.get(name)?.equals(bytes));
  report.changedOriginalFiles = changed.length;
  report.newFiles = [...after.keys()].filter(name => !baseline.has(name)).length;
  assert.equal(report.newFiles, 1, "only the intentional page create may add a file");
  assert.equal(changed.length, 0, "a pre-existing file changed beyond the synthetic blocks");
  report.formats = org ? ["markdown", "org"] : ["markdown"];
  report.doctorAfter = doctor();
  assert.equal(report.doctorAfter.exit, 0, "doctor must be clean after acceptance");
  // Warm up once, then measure actual process startup + discovery + handshake +
  // host planning, guarded save/fsync and reply, with a fresh CLI per operation.
  cycle(created, 1000, false);
  for (let i = 0; i < 30; i++) cycle(created, i + 2000, true);
  report.budgetMs = Object.fromEntries(Object.entries(timings).map(([verb, times]) => {
    times.sort((a,b) => a-b);
    return [verb, { runs: times.length, median: times[15], p95: times[28] }];
  }));
} finally {
  spawnSync(app, ["serve", graph, "--stop"], { env, encoding: "utf8", timeout: 60_000 });
  fs.writeFileSync(path.join(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
}
console.log(JSON.stringify(report, null, 2));
