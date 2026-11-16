// Real owner acceptance for CLI page operations and lazy reference helpers.
// Usage: node scripts/cli-page-acceptance.mjs APP NEW_ARTIFACT_DIRECTORY
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const [binary, output] = process.argv.slice(2);
assert(binary && output, "supply APP and a new artifact directory");
const app = path.resolve(binary), root = path.resolve(output);
assert(!fs.existsSync(root), "use a fresh scratch directory");
fs.mkdirSync(root, { recursive:true });
const transcript = [];
const commit = spawnSync("git", ["rev-parse", "HEAD"], {encoding:"utf8"}).stdout.trim();
const owners = [];
try {
  for (const format of ["md", "org"]) {
    const base = path.join(root, format), graph = path.join(base, "graph");
    for (const sub of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, sub), { recursive:true });
    const prefix = format === "org" ? "* " : "- ";
    fs.writeFileSync(path.join(graph, "logseq/config.edn"), `{:file/name-format :triple-lowbar :preferred-format "${format === "org" ? "Org" : "Markdown"}" :default-home {:page "Home"}}\n`);
    fs.writeFileSync(path.join(graph, `pages/Home.${format}`), `${prefix}home\n`);
    fs.writeFileSync(path.join(graph, `pages/Ghost___child.${format}`), `${prefix}child\n`);
    fs.writeFileSync(path.join(graph, `pages/FilelessLinks.${format}`), `${prefix}[[RefOnly]] [[Ghost]] [[Ghost/child]] [[Home]]\n`);
    fs.writeFileSync(path.join(graph, `pages/different.${format}`), format === "org"
      ? "#+TITLE: Project\n#+ALIAS: Shortcut\n* target\n:PROPERTIES:\n:author: Martin\n:END:\n"
      : "title:: Project\nalias:: Shortcut\n\n- target\n  author:: Martin\n");
    fs.writeFileSync(path.join(graph, `pages/Project___child.${format}`), `${prefix}child\n`);
    fs.writeFileSync(path.join(graph, `pages/Links.${format}`), `${prefix}[[Project]] [[Project/child]]\n`);
    const env = { ...process.env, TINE_RUN:"", TINE_AUTHOR:"slice3", XDG_DATA_HOME:path.join(base,"data"), XDG_CONFIG_HOME:path.join(base,"config"), XDG_CACHE_HOME:path.join(base,"cache"), XDG_STATE_HOME:path.join(base,"state") };
    const log = fs.openSync(path.join(base,"serve.log"), "w");
    const owner = spawn(app, ["serve",graph], { env,stdio:["ignore",log,log] });
    fs.closeSync(log);
    owners.push({owner,graph,env});
    function cli(args, expected = 0, json = true) {
      const result = spawnSync(app, [...args,"--graph",graph,...(json ? ["--json"] : [])], {env,encoding:"utf8",timeout:30_000});
      assert.equal(result.status, expected, `${args.join(" ")}: ${result.stderr}`);
      const reply = json ? JSON.parse(expected ? result.stderr : result.stdout) : result.stdout;
      transcript.push({format,args,exit:result.status,reply});
      return reply;
    }
    // Wait for the explicitly launched serve; status may connect while it starts.
    for (let tries=0; tries<100; tries++) {
      assert(owner.exitCode === null, "serve exited before readiness");
      const result = spawnSync(app,["status","--graph",graph,"--json"],{env,encoding:"utf8",timeout:2000});
      if (result.status === 0) break;
      await sleep(100);
    }
    assert.equal(cli(["status"]).owner,"serve");
    cli(["page","rename","RefOnly","RenamedRef"]);
    cli(["page","rename","Ghost","NewGhost"]);
    assert.equal(cli(["page","show","FilelessLinks"]).page.blocks[0].raw,"[[RenamedRef]] [[NewGhost]] [[NewGhost/child]] [[Home]]");
    assert(fs.existsSync(path.join(graph, `pages/NewGhost___child.${format}`)));
    for (const name of ["RefOnly", "RenamedRef", "Ghost", "NewGhost"]) assert(!fs.existsSync(path.join(graph, `pages/${name}.${format}`)));
    fs.mkdirSync(path.join(graph,"logseq/.tine-trash/pages"),{recursive:true});
    fs.chmodSync(path.join(graph,"logseq"),0o555);
    let partial;
    try { partial = cli(["page","rename","Home","MovedHome"],8); }
    finally { fs.chmodSync(path.join(graph,"logseq"),0o755); }
    assert.equal(partial.error.code,"uncertain");
    assert.equal(partial.error.detail.admitted,true);
    assert.equal(partial.error.detail.report.outcome,"renamed");
    assert(fs.existsSync(path.join(graph, `pages/MovedHome.${format}`)));
    assert(!fs.existsSync(path.join(graph, `pages/Home.${format}`)));
    assert(fs.readFileSync(path.join(graph,`pages/FilelessLinks.${format}`),"utf8").includes("[[MovedHome]]"));
    assert(fs.readFileSync(path.join(graph,"logseq/config.edn"),"utf8").includes(':page "Home"'));
    const created = cli(["page","create","Empty"]);
    assert(created.page.endsWith(`.${format}`));
    cli(["page","create","Empty"],6);
    const page = cli(["page","show","Shortcut"]);
    const handle = `${page.page.name}/${page.page.blocks[0].handle}`;
    const ref = cli(["ref",handle]);
    const embed = cli(["embed",ref.uuid]);
    assert.equal(embed.text,`{{embed ((${ref.uuid}))}}`);
    assert.equal(cli(["ref",ref.uuid],0,false),`((${ref.uuid}))\n`);
    const persisted = fs.readFileSync(path.join(graph,`pages/different.${format}`),"utf8");
    assert(persisted.includes(ref.uuid));
    assert(persisted.includes(format === "org" ? ":author: Martin" : "author:: Martin"));
    cli(["page","rename","Shortcut","project"]);
    const renamed = cli(["page","rename","project","New/Project"]);
    assert.equal(renamed.report.outcome,"renamed");
    cli(["page","show","New/Project/child"]);
    assert.equal(cli(["page","show","Links"]).page.blocks[0].raw,"[[New/Project]] [[New/Project/child]]");
    cli(["page","rename","New/Project","Empty"],6);
    cli(["page","delete","Empty"],6);
    const stale = cli(["page","delete","Empty","--yes","--rev","old"],6);
    assert.equal(stale.error.code,"stale");
    cli(["add",`((${ref.uuid}))`,"--page","Links"]);
    const deleted = cli(["page","delete","New/Project","--yes"]);
    assert.equal(deleted.broken_refs,2);
    cli(["page","show","New/Project"],3);
    assert.equal(cli(["page","delete","Empty","--force"],0,false),"deleted page Empty (breaks 0 references)\n");
    const stopped = spawnSync(app,["serve",graph,"--stop"],{env,encoding:"utf8",timeout:30_000});
    assert.equal(stopped.status,0,stopped.stderr);
  }
  fs.writeFileSync(path.join(root,"receipt.json"),JSON.stringify({commit,passed:true,formats:["md","org"],verbs:["page create","page rename","page delete","ref","embed"],transcript},null,2)+"\n");
  console.log("PASS: five verbs against real serve owners, Markdown and Org");
} catch (error) {
  fs.writeFileSync(path.join(root,"failure.json"),JSON.stringify({commit,journey:"CLI page and helper verbs",expected:"durable owner operations in both formats",observed:String(error),classification:"ambiguous",transcript},null,2)+"\n");
  throw error;
} finally {
  for (const {owner,graph,env} of owners) {
    spawnSync(app,["serve",graph,"--stop"],{env,encoding:"utf8",timeout:30_000});
    if (owner.exitCode === null) owner.kill("SIGTERM");
  }
}
