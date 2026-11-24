// Prepare the published storage fixture and enumerate finite schedules with Quint 0.33.0.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const storage = path.resolve(__dirname, "../../storage-model/quint");
const out = path.resolve(__dirname, "../../../target/compose-stage4");
const fixture = fs.readFileSync(path.join(storage, "storage-s36.qnt"), "utf8");
const scenarios = fs.readFileSync(path.join(storage, "scenarios-s36.inc"), "utf8");
const composition = fs.readFileSync(path.join(__dirname, "finish.inc"), "utf8");
const hook = "  // @@SCENARIOS@@";
const slots = "pure val PAGES = Set(0, 1, 2)";
assert.equal(fixture.split(hook).length, 2);
assert.equal(fixture.split(slots).length, 2);

if (process.argv[2] === "generate") {
  fs.mkdirSync(out, { recursive: true });
  for (const [file, source] of [
    ["finish", fixture.replace(hook, () => scenarios + "\n" + composition)],
    ["shared-referrer", fixture.replace(slots, "pure val PAGES = Set(0, 1, 2, 3, 4)")
      .replace(hook, () => scenarios + "\n" + composition)],
    ["storage-base", fixture.replace(hook, () => scenarios)],
  ]) {
    fs.writeFileSync(path.join(out, file + ".qnt"), source);
  }
  return;
}

const modules = path.join(storage, "node_modules");
const quint = (name) => require(path.join(modules, "@informalsystems/quint/dist/src", name));
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

const poll = "poll";
const idle = "idle";
const crash = "stopHost";
const fail = ["failWrite", "failSync"];
const power = "powerReset";
const scans = [
  // Each graph lists all allowed choices for that fixture. The last column
  // selects a fairness assumption to drop or a different driver.
  ["clean", "admitted", 16, [poll, idle, ...fail]],
  ["no-io-fairness", "admitted", 16, [poll, idle, ...fail], "allowFailure"],
  ["no-service", "noResume", 16, [poll, idle], "allowIdle"],
  ["crash-before-sync", "interrupted", 18, [poll, idle, crash, power]],
  ["repeated-launch", "repeated", 16, [poll, idle, crash]],
  ["permanent-name-claim", "claimed", 10, [poll, idle, crash, ...fail]],
  ["retry-only", "claimed", 10, ["pollRetryOnly", idle]],
  ["claim-churn", "claimed", 17, [poll, idle, "claimArrives", "claimLeaves"]],
  ["foreign-write", "preparedConflict", 10, [poll, idle, "rewriteDestination", "deleteDestination"]],
  ["foreign-write-at-sync", "syncRaced", 10, [poll, idle]],
  ["stale-submit", "stale", 10, [poll, idle, "syncStaleDraft"]],
  ["cancel", "cancellable", 17, [poll, idle, "cancel"]],
  ["cancel-crash", "interruptedCancel", 17, [poll, idle]],
  ["starvation-mutant", "admitted", 16, [poll, idle, "pollSkip"]],
  ["premature-success-mutant", "admitted", 16, ["pollEarly", idle]],
];
const selection = process.argv[2];
const selected = selection ? scans.filter(([name]) => name === selection) : scans;
assert(selected.length, "Unknown scan");
const modelPath = path.join(out, "finish.qnt");
const resolver = fileSourceResolver();
const wrappers = `
  action powerReset = cutPower(Set())
  action claimArrives = changeClaim(true)
  action claimLeaves = changeClaim(false)
  action rewriteDestination = foreignWrite(2, 2, true)
  action deleteDestination = foreignWrite(2, ABSENT, true)
`;
const source = fs.readFileSync(modelPath, "utf8").trimEnd();
assert(source.endsWith("}"));
const parsed = parse(newIdGenerator(), modelPath, resolver.lookupPath(out, "finish.qnt"),
  source.slice(0, -1) + wrappers + "}\n");
assert.deepEqual(parsed.errors, []);
const definitions = new Map(parsed.modules[0].declarations.filter(d => d.kind === "def").map(d => [d.name, d.expr]));
const builder = new Builder(parsed.table, false);
const rng = newRng(1n);
// Nondeterministic environment choices are individual graph edges.
const ctx = new Context(newTraceRecorder(0, rng), () => { throw Error("Nondeterministic checker action"); }, builder.varStorage);
const evaluate = (name) => {
  assert(definitions.has(name), name);
  const evaluateExpression = buildExpr(builder, definitions.get(name));
  return () => evaluateExpression(ctx).unwrap().toBool();
};
const checks = new Map(["pending", "finishSafety", "noPrematureSuccess", "ok"].map(name => [name, evaluate(name)]));
const outcome = (state) => state.get("request").toOrderedMap().get("outcome").toStr();

function seedState(seed) {
  builder.varStorage.reset();
  ctx.trace.reset();
  assert(evaluate(seed)(), seed);
  ctx.varStorage.shiftVars();
  return ctx.varStorage.asRecord().normalForm();
}

function period(seed, actions, counter) {
  let state = seedState(seed);
  const first = state.toOrderedMap();
  const counterValue = (record) => record.get("g").toOrderedMap().get(counter);
  const eraseCounter = (record) => record.set("g", record.get("g").toOrderedMap().delete(counter));
  const steps = actions.map(evaluate);
  for (let n = 0; n < 2; n++) {
    for (const step of steps) {
      ctx.varStorage.fromRecord(state);
      assert(step());
      ctx.varStorage.shiftVars();
      state = ctx.varStorage.asRecord().normalForm();
      assert(checks.get("pending")() && checks.get("finishSafety")());
    }
    const after = state.toOrderedMap();
    assert(eraseCounter(first).equals(eraseCounter(after)), counter);
    assert(!counterValue(first).equals(counterValue(after)), counter);
  }
  console.log(`Repeatable prefix: ${seed}, two rounds of ${actions.join(", ")}; only g.${counter} advances`);
}

if (!selection || selection === "repeated-launch") period("noResume", [poll, crash], "vc");
if (!selection || selection === "foreign-write") period("afterWriterRound",
  [poll, "rewriteDestination", poll, "deleteDestination", poll], "ext");

if (!selection || selection === "cancel-crash") {
  const withDiscard = seedState("interruptedCancel").toOrderedMap();
  const withoutDiscard = seedState("noResume").toOrderedMap();
  // Model ghost history differs. The entire concrete storage state after down coincides.
  assert(withDiscard.get("s").equals(withoutDiscard.get("s")));
  assert.equal(outcome(withDiscard), "Cancelling");
  assert.equal(outcome(withoutDiscard), "Pending");
  console.log("Cancellation: post-crash storage states identical with and without delivered Discard");
}

function search([name, seed, depth, names, policy]) {
  const root = seedState(seed);
  const graph = [{ state: root, depth: 0, parent: null, via: null, edges: [] }];
  const index = StateMap([[root, 0]]).asMutable();
  const actions = names.map(action => [action, evaluate(action)]);
  const outcomes = {};
  let edges = 0;
  let expanded = 0;
  let safety = null;
  const restore = (state) => ctx.varStorage.fromRecord(state);
  const prefix = (i) => {
    const path = [];
    for (let node = graph[i]; node.parent !== null; node = graph[node.parent]) path.push(node.via);
    return path.reverse();
  };
  const show = index => expressionToString(rv.toQuintEx(graph[index].state));

  for (let i = 0; i < graph.length; i++) {
    const node = graph[i];
    restore(node.state);
    const label = outcome(node.state.toOrderedMap());
    outcomes[label] = (outcomes[label] ?? 0) + 1;
    node.pending = checks.get("pending")();
    if (!checks.get("finishSafety")() && safety === null) {
      safety = { trace: prefix(i), noPrematureSuccess: checks.get("noPrematureSuccess")(),
        storageSafety: checks.get("ok")(), state: show(i) };
    }
    if (!node.pending || node.depth === depth) continue;
    expanded++;
    for (const [action, enabled] of actions) {
      restore(node.state);
      if (!enabled()) continue;
      ctx.varStorage.shiftVars();
      const next = ctx.varStorage.asRecord().normalForm();
      assert.equal(next.toOrderedMap().size, 3, "An action left a variable unset");
      if (!index.has(next)) {
        index.set(next, graph.length);
        graph.push({ state: next, depth: node.depth + 1, parent: i, via: action, edges: [] });
      }
      node.edges.push([index.get(next), action]);
      edges++;
    }
  }

  const pollEdges = new Set([poll, "pollSkip", "pollRetryOnly", "pollEarly"]);
  const allowed = (target, action) => graph[target].pending && action !== crash && action !== power
    && (policy === "allowFailure" || !fail.includes(action));
  const required = action => policy === "allowIdle" || pollEdges.has(action);
  const frontier = graph.filter(node => node.pending && node.depth === depth).length;
  // For each eligible first edge, find the shortest path back. A loop may be
  // rotated to put its fair poll first. Bounds are separate for prefix and loop.
  function lasso() {
    for (let from = 0; from < graph.length; from++) {
      for (const [first, action] of graph[from].edges) {
        if (!required(action) || !allowed(first, action)) continue;
        const queue = [{ at: first, loop: [action] }];
        const seen = new Set([first]);
        for (const { at, loop } of queue) {
          if (at === from) return { prefix: prefix(from), loop, state: show(from) };
          if (loop.length === depth) continue;
          for (const [next, label] of graph[at].edges) {
            if (!seen.has(next) && allowed(next, label)) {
              seen.add(next);
              queue.push({ at: next, loop: [...loop, label] });
            }
          }
        }
      }
    }
    return null;
  }

  const cycle = lasso();
  const result = { name, seed, depth, states: graph.length, expanded, frontier, edges, outcomes, safety, cycle };
  fs.writeFileSync(path.join(out, name + ".json"), JSON.stringify(result, null, 2) + "\n");
  console.log(JSON.stringify({ ...result,
    safety: safety && { ...safety, state: undefined },
    cycle: cycle && { ...cycle, state: undefined } }));
  return result;
}

for (const scenario of selected) {
  const result = search(scenario);
  const expectedCycle = ["no-io-fairness", "no-service", "retry-only", "stale-submit", "cancel-crash", "starvation-mutant"];
  assert.equal(Boolean(result.cycle), expectedCycle.includes(result.name), result.name);
  assert.equal(Boolean(result.safety), result.name === "premature-success-mutant", result.name);
  if (result.safety) {
    assert.equal(result.safety.noPrematureSuccess, false);
    assert.equal(result.safety.storageSafety, true);
  }
}
