import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { graphEpoch, setGraphMeta } from "./graphSession";
import { changeJournalTitleFormat } from "./ui";

afterEach(() => { setGraphMeta(null); vi.restoreAllMocks(); });

it("does not publish an old graph's journal migration completion into the new graph", async () => {
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("no graph metadata");
  setGraphMeta({ ...loaded.meta, root: "/old", journal_page_title_format: "Old" });
  let finish!: (result: { migrated: number; skipped: [] }) => void;
  vi.spyOn(backend(), "setJournalTitleFormat").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  changeJournalTitleFormat("Changed");
  setGraphMeta({ ...loaded.meta, root: "/new", journal_page_title_format: "New" });
  const before = graphEpoch();
  finish({ migrated: 0, skipped: [] });
  await Promise.resolve();
  await Promise.resolve();
  expect(graphEpoch()).toBe(before);
});
