// Usage: node scripts/cli-move-indent-acceptance.mjs APP NEW_ARTIFACT_DIRECTORY
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { accountHomeFixture } from "./lib/e2e-account-home.mjs";

const [binary, output] = process.argv.slice(2);
assert(binary && output, "supply APP and a new artifact directory");
const app = path.resolve(binary), root = path.resolve(output);
assert(!fs.existsSync(root), "use a fresh artifact directory");
const graph = path.join(root, "graph");
fs.mkdirSync(path.join(graph, "pages"), { recursive: true });
fs.mkdirSync(path.join(graph, "logseq"));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:file/name-format :triple-lowbar}\n");
const corpus = JSON.parse(fs.readFileSync(new URL("../src-tauri/src/owner/owner_tests/write-transcripts.json", import.meta.url), "utf8"));
const cases = corpus.filter(row => /-move-3-to-|^md-mixed-prefix-/.test(row.case));
assert.equal(cases.length, 66);
const files = cases.map((row, i) => {
  const name = `move-${i}`, file = path.join(graph, "pages", `${name}.${row.case.startsWith("org-") ? "org" : "md"}`);
  fs.writeFileSync(file, row.input);
  return { row, name, file };
});
const fixture = accountHomeFixture(root, root, { ...process.env, HOME: root,
  TINE_RUN: "", TINE_AUTHOR: "", XDG_DATA_HOME: path.join(root, "data"),
  XDG_CONFIG_HOME: path.join(root, "config"), XDG_CACHE_HOME: path.join(root, "cache"),
  XDG_STATE_HOME: path.join(root, "state") });
const env = fixture.env;
const commit = spawnSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).stdout.trim();
const log = fs.openSync(path.join(root, "serve.log"), "w");
const owner = spawn(app, ["serve", graph], { env, stdio: ["ignore", log, log] });
fs.closeSync(log);
const transcript = [];
const cli = args => spawnSync(app, [...args, "--graph", graph, "--json"], { env, encoding: "utf8", timeout: 30_000 });
try {
  const deadline = Date.now() + 20_000;
  while (true) {
    assert.equal(owner.exitCode, null, "serve exited before readiness");
    if (cli(["status"]).status === 0) break;
    assert(Date.now() < deadline, "serve did not become ready");
    await sleep(50);
  }
  for (const { row, name, file } of files) {
    const args = row.command.slice(1, -2).map(arg => arg.replace(/^proof\//, `${name}/`));
    const result = cli(args);
    const after = fs.readFileSync(file, "utf8");
    transcript.push({ case: row.case, args, exit: result.status, stdout: result.stdout, stderr: result.stderr, input: row.input, expected: row.after, after });
    assert.equal(result.status, 0, `${row.case}: ${result.stderr}`);
    assert.equal(after, row.after, row.case);
    const reread = cli(["page", "show", name]);
    assert.equal(reread.status, 0, `${row.case}: reread ${reread.stderr}`);
  }
  fs.writeFileSync(path.join(root, "receipt.json"), JSON.stringify({ commit, passed: true, cases: transcript.length, transcript }, null, 2) + "\n");
  console.log("PASS: 64 shallower CLI moves and 2 mixed-prefix edits preserve exact Markdown/Org bytes");
} catch (error) {
  fs.writeFileSync(path.join(root, "failure.json"), JSON.stringify({ commit, journey: "CLI move subtree to shallower depth", expected: "exact descendant indentation, bodies and drawers in saved files", observed: String(error), classification: transcript.at(-1)?.exit === 0 ? "product" : "ambiguous", transcript }, null, 2) + "\n");
  throw error;
} finally {
  cli(["serve", graph, "--stop"]);
  if (owner.exitCode === null) owner.kill("SIGTERM");
  fixture.close?.();
  fixture.verify({ requireLease: false });
}
