const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { model, output, storage } = require("./batch-model.cjs");

const slots = [0, 1, 2, 3, 4];
const wrappers = new Map();
const wrap = (name, expression) => { wrappers.set(name, expression); return name; };
const nativeSave = ["batchCheck", "batchRename", "batchSync", ...slots.map(p =>
  wrap(`saveSlot${p}`, `if (s.pages.get(${p}).buf == ABSENT) flushDel(${p}) else beginWrite(${p})`))];
const discard = [wrap("deliverSuccess", "deliverUp(true)"), wrap("deliverFailure", "deliverUp(false)"),
  ...slots.flatMap(p => [wrap(`queue${p}`, `queueDiscard(${p})`), wrap(`reply${p}`, `receiveReply(${p})`)])];
const power = Array.from({ length: 2 ** slots.length }, (_, mask) =>
  wrap(`power${mask}`, `selectedPower(Set(${slots.filter(p => mask & (1 << p)).join(", ")}))`));
const outside = slots.flatMap(p => ["ABSENT", "3"].flatMap((bytes, i) => [true, false].map(durable =>
  wrap(`external${p}_${i}_${durable}`, `extWriteD(${p}, ${bytes}, ${durable})`))));
const claims = [2, 4].flatMap(p => [true, false].map(present =>
  wrap(`claim${p}_${present}`, `extClaim(${p}, ${present})`)));
const cleanup = slots.map(p => wrap(`retire${p}`, `draftSync(${p})`));
const observations = slots.map(p => wrap(`observe${p}`, `observe(${p})`));
const mixed = [...nativeSave, ...discard, ...power, ...outside, ...claims, ...cleanup, ...observations,
  "crash", "launch", "saveFail", wrap("syncFail", "dirSync(false)")];

const scans = [
  { name: "clean", seed: "batchAdmitted", depth: 20, actions: nativeSave },
  { name: "mixed", seed: "batchAdmitted", depth: 3, actions: mixed },
  { name: "after-first", seed: "firstReceiver", depth: 3, actions: mixed },
  { name: "source-cut", seed: "liveRemoval", depth: 3, actions: mixed },
  { name: "recovery", seed: wrap("recoveredBetween", "firstReceiver.then(crash).then(launch)"), depth: 11,
    actions: [...nativeSave, ...cleanup, ...observations] },
  { name: "per-pair", seed: "batchAdmitted", depth: 0, actions: [], fails: "gateOwnership" },
  { name: "early-source", seed: "batchAdmitted", depth: 1, actions: nativeSave, fails: "orderHolds" },
  { name: "discard-both", seed: "firstReceiver", depth: 2, actions: discard, fails: "discardIsolation" },
];
const extra = "\n" + [...wrappers].map(([name, expr]) => `  action ${name} = ${expr}\n`).join("");

const selection = process.argv[2];
if (selection === "generate") {
  fs.mkdirSync(output, { recursive: true });
  fs.writeFileSync(path.join(output, "batch.qnt"), model("none", extra));
  return;
}

const modules = path.join(storage, "node_modules");
const quint = name => require(path.join(modules, "@informalsystems/quint/dist/src", name));
assert.equal(require(path.join(modules, "@informalsystems/quint/package.json")).version, "0.33.0");
const { Map: StateMap } = require(path.join(modules, "immutable"));
const { buildExpr, Builder } = quint("runtime/impl/builder");
const { Context } = quint("runtime/impl/Context");
const { newIdGenerator } = quint("idGenerator");
const { newRng } = quint("rng");
const { newTraceRecorder } = quint("runtime/trace");
const { fileSourceResolver } = quint("parsing/sourceResolver");
const { parse } = quint("parsing/quintParserFrontend");
const { expressionToString } = quint("ir/IRprinting");
const { rv } = quint("runtime/impl/runtimeValue");

function search({ name, seed, depth, actions, fails }) {
  const filename = path.join(output, `${name}.qnt`);
  const resolver = fileSourceResolver();
  const source = model(fails ? name : "none", extra);
  fs.writeFileSync(filename, source);
  const parsed = parse(newIdGenerator(), filename, resolver.lookupPath(output, path.basename(filename)), source);
  assert.deepEqual(parsed.errors, []);
  const definitions = new Map(parsed.modules[0].declarations.filter(d => d.kind === "def").map(d => [d.name, d.expr]));
  const builder = new Builder(parsed.table, false);
  const ctx = new Context(newTraceRecorder(0, newRng(1n)), () => { throw Error("Nondeterministic action"); }, builder.varStorage);
  const evaluate = name => {
    assert(definitions.has(name), name);
    const expr = buildExpr(builder, definitions.get(name));
    return () => {
      try { return expr(ctx).unwrap().toBool(); }
      catch (cause) { throw Error(`Evaluating ${name}`, { cause }); }
    };
  };
  const checks = ["guarantee", "orderHolds", "renameResolves", "gateOwnership", "batchFootprintHolds", "requirementsBlock",
    "discardIsolation", "removedCustody"].map(name => [name, evaluate(name)]);
  const moves = actions.map(name => [name, evaluate(name)]);
  assert(evaluate(seed)(), seed);
  ctx.varStorage.shiftVars();
  const root = ctx.varStorage.asRecord().normalForm();
  const graph = [{ state: root, depth: 0, parent: -1, via: seed }];
  const seen = StateMap([[root, 0]]).asMutable();
  const violations = {};
  let edges = 0;
  let frontier = 0;
  function prefix(i) {
    const steps = [];
    for (; i !== -1; i = graph[i].parent) steps.push(graph[i].via);
    return steps.reverse();
  }
  for (let i = 0; i < graph.length; i++) {
    const node = graph[i];
    ctx.varStorage.fromRecord(node.state);
    for (const [label, holds] of checks) {
      if (!holds() && !violations[label]) {
        violations[label] = { trace: prefix(i), state: expressionToString(rv.toQuintEx(node.state)) };
      }
    }
    if (node.depth === depth) { frontier++; continue; }
    for (const [via, enabled] of moves) {
      ctx.varStorage.fromRecord(node.state);
      if (!enabled()) continue;
      ctx.varStorage.shiftVars();
      const state = ctx.varStorage.asRecord().normalForm();
      assert.equal(state.toOrderedMap().size, 2, via);
      edges++;
      if (seen.has(state)) continue;
      seen.set(state, graph.length);
      graph.push({ state, depth: node.depth + 1, parent: i, via });
    }
  }
  const result = { name, depth, states: graph.length, frontier, edges, violations };
  fs.writeFileSync(path.join(output, `${name}.json`), JSON.stringify(result, null, 2) + "\n");
  console.log(JSON.stringify({ ...result, violations: Object.fromEntries(
    Object.entries(violations).map(([label, { trace }]) => [label, trace])) }));
  return fails ? Boolean(violations[fails]) : Object.keys(violations).length === 0;
}

const selected = scans.filter(scan => !selection || scan.name === selection);
assert(selected.length, `Unknown scan ${selection}`);
fs.mkdirSync(output, { recursive: true });
for (const scan of selected) if (!search(scan)) process.exitCode = 1;
