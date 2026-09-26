import { describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { carryDay } from "./carry";
import { journalTitle } from "./journal";
import { doc, loadSingle, pageByName, resetStore } from "./store";
import type { PageRead } from "./types";

describe("carry binding", () => {
  it("does not load or write an old day when today's read finishes after a graph switch (I-20)", async () => {
    resetStore();
    let finish!: (page: PageRead | null) => void;
    const read = vi.spyOn(backend(), "getPage").mockImplementationOnce(() =>
      new Promise((resolve) => { finish = resolve; })
    );
    const save = vi.spyOn(backend(), "savePage");
    const carrying = carryDay("2026-09-25");
    await vi.waitFor(() => expect(read).toHaveBeenCalledWith(journalTitle(new Date()), "journal"));
    resetStore();
    loadSingle({ name: "New graph", kind: "page", title: "New graph", pre_block: null, blocks: [] });
    finish({ name: journalTitle(new Date()), kind: "journal", title: "Today", id: "journals/old.md", pre_block: null, blocks: [] });
    await carrying;
    expect(doc.feed).toEqual(["New graph"]);
    expect(pageByName(journalTitle(new Date()))).toBeUndefined();
    expect(save).not.toHaveBeenCalled();
    read.mockRestore();
    save.mockRestore();
  });
});
