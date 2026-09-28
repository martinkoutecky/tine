import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { resetStore } from "./document";
import { graphMeta, setGraphMeta, setJournalTemplate } from "./graphSession";

afterEach(() => { setGraphMeta(null); vi.restoreAllMocks(); });

it("does not roll back the new graph's template when an old config write fails", async () => {
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("no graph metadata");
  setGraphMeta({ ...loaded.meta, root: "/old", default_journal_template: "Old" });
  let fail!: (error: Error) => void;
  vi.spyOn(backend(), "setDefaultJournalTemplate").mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
  setJournalTemplate("Changed");
  resetStore();
  setGraphMeta({ ...loaded.meta, root: "/new", default_journal_template: "New" });
  fail(new Error("old graph disk error"));
  await Promise.resolve();
  await Promise.resolve();
  expect(graphMeta()?.default_journal_template).toBe("New");
});
