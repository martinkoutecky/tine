// Guard (og storage.qnt mutant MX; STEP3-DESIGN §6): a graph switch goes on only
// after the old graph's input has custody: published by its page host (the
// awaited `flushAll` barrier, aborting the switch when it fails), or kept by the
// host's crash draft. Threat: an edit typed while load_graph ran has no copy
// when the working set is reset. Step 3b P2b deleted the v1 draft store
// (`keepAtSwitch`, D-1); the page host owns crash drafts, and §6 step 5 says
// nothing closes over missing custody. Behaviour: src/graph.test.tsx "MX:"
// tests. Exemplar: loadGraphPath in src/graph.ts.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const graph = readFileSync(new URL("./graph.ts", import.meta.url), "utf8");
const RULE = "a graph switch resets the old working set only after an awaited publication barrier, and never over input "
  + "the old host has not taken (storage.qnt mutant MX; STEP3-DESIGN §6 step 5)";

describe("graph switch keeps the old graph's late edits", () => {
  it("loadGraphPath awaits the last flushAll barrier, aborting on failure, before load_graph and resetStore", () => {
    const flush = graph.indexOf("if (hadGraph && !(await flushAll())) {");
    const abort = graph.indexOf("return { kind: \"aborted\" };", flush);
    const load = graph.indexOf("await backend().loadGraph(path)", flush);
    const reset = graph.indexOf("resetStore();", load);
    expect(flush, RULE).toBeGreaterThan(0);
    expect(abort > flush && abort < load && load < reset, RULE).toBe(true);
    expect(graph, RULE).not.toMatch(/void flushAll\(\)/);
  });

  it("loadGraphPath does not reset the working set over input its old host never answered", () => {
    const load = graph.indexOf("await backend().loadGraph(path)");
    const reset = graph.indexOf("resetStore();", load);
    const between = graph.slice(load, reset);
    const late = between.indexOf("unsavedDrafts()");
    // Late input found after load_graph must keep custody: the switch stops
    // (or the input is held) before resetStore, never dropped with a notice.
    expect(late < 0 || /\breturn\b/.test(between.slice(late)), RULE).toBe(true);
  });
});
