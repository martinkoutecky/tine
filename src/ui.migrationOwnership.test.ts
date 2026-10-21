import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { bumpGraphEpoch, graphEpoch, graphMeta, setGraphMeta } from "./graphSession";
import { changeJournalTitleFormat } from "./ui";

afterEach(() => { setGraphMeta(null); vi.restoreAllMocks(); });

it("does not publish an old graph's journal migration completion into the new graph", async () => {
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("no graph metadata");
  setGraphMeta({ ...loaded.meta, root: "/old", journal_page_title_format: "Old" });
  let finish!: (notes: string[]) => void;
  vi.spyOn(backend(), "setJournalTitleFormat").mockImplementationOnce(() => new Promise<string[]>((resolve) => { finish = resolve; }));
  changeJournalTitleFormat("Changed");
  setGraphMeta({ ...loaded.meta, root: "/new", journal_page_title_format: "New", journal_config_diagnostics: ["new graph note"] });
  const before = graphEpoch();
  finish(["old graph note"]);
  await Promise.resolve();
  await Promise.resolve();
  expect(graphEpoch()).toBe(before);
  expect(graphMeta()?.journal_config_diagnostics).toEqual(["new graph note"]);
});

it("does not roll back a newer same-value journal format request", async () => {
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("no graph metadata");
  setGraphMeta({ ...loaded.meta, root: "/graph", journal_page_title_format: "F" });
  let fail!: (reason: Error) => void;
  vi.spyOn(backend(), "setJournalTitleFormat")
    .mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }))
    .mockResolvedValue([]);
  changeJournalTitleFormat("X");
  changeJournalTitleFormat("F");
  changeJournalTitleFormat("X");
  fail(new Error("old write failed"));
  await Promise.resolve();
  await Promise.resolve();
  expect(graphMeta()?.journal_page_title_format).toBe("X");
});

it("does not roll back a journal format request after graph A is reopened", async () => {
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("no graph metadata");
  setGraphMeta({ ...loaded.meta, root: "/graph-a", journal_page_title_format: "F" });
  let fail!: (reason: Error) => void;
  vi.spyOn(backend(), "setJournalTitleFormat").mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
  changeJournalTitleFormat("X");
  setGraphMeta({ ...loaded.meta, root: "/graph-b", journal_page_title_format: "B" });
  bumpGraphEpoch();
  setGraphMeta({ ...loaded.meta, root: "/graph-a", journal_page_title_format: "X" });
  bumpGraphEpoch();
  fail(new Error("old graph write failed"));
  await Promise.resolve();
  await Promise.resolve();
  expect(graphMeta()?.journal_page_title_format).toBe("X");
});
