// Guard (og storage.qnt mutant MX): a graph switch goes on only after the old
// graph's late edits are durable in its draft store, or are held in this
// window (switchHeldDrafts) with a sticky error saying so. Threat: an edit
// typed while load_graph ran has no durable copy when the working set is reset;
// a draft-store refusal (disk error, the 64-page / 8 MiB bound) used to drop it
// with only a toast. Behaviour: src/graph.test.tsx "MX:" tests and
// src/unsavedRecovery.test.tsx. Exemplar: loadGraphPath in src/graph.ts.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const graph = readFileSync(new URL("./graph.ts", import.meta.url), "utf8");
const store = readFileSync(new URL("./draftStore.ts", import.meta.url), "utf8");
const RULE = "a graph switch awaits keepAtSwitch before it goes on, and a refused draft stays held in the window (storage.qnt mutant MX)";

describe("graph switch keeps the old graph's late edits", () => {
  it("loadGraphPath awaits keepAtSwitch right after resetStore, before the new graph's state", () => {
    const at = graph.indexOf("keepAtSwitch(oldRoot)");
    const reset = graph.indexOf("resetStore();", at);
    const awaited = graph.indexOf("await kept", reset);
    const next = graph.indexOf("clearWorkspaces();", reset);
    expect(at, RULE).toBeGreaterThan(0);
    expect(reset > at && awaited > reset && awaited < next, RULE).toBe(true);
    expect(graph, RULE).not.toMatch(/void kept/);
  });

  it("keepAtSwitch hands the snapshot to the window holder before any write, and releases a record only after its write", () => {
    const body = store.slice(store.indexOf("export function keepAtSwitch"), store.indexOf("/** Retire a record an earlier session kept"));
    expect(body.indexOf("setHeld("), RULE).toBeGreaterThan(0);
    expect(body.indexOf("setHeld("), RULE).toBeLessThan(body.indexOf("storeDraft"));
    expect(body.indexOf("dismissHeldDraft(record.id)"), RULE).toBeGreaterThan(body.indexOf("storeDraft"));
  });
});
