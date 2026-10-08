import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { resetStore, setRaw } from "./document";
import { loadRoutedPage } from "./document/workingSet";

afterEach(() => { resetStore(); vi.restoreAllMocks(); });

it("saves a page routed directly after startup, without first opening the journal feed", async () => {
  resetStore();
  const write = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["next-rev"] });
  loadRoutedPage({ name: "Routed", kind: "page", title: "Routed", id: "pages/Routed.md",
    rev: "base-rev", pre_block: null, blocks: [
      { id: "block", raw: "id:: 33333333-3333-4333-8333-333333333333", collapsed: false, children: [] },
    ] });
  setRaw("block", "[[Fuzzy Existing]] \nid:: 33333333-3333-4333-8333-333333333333", { timetracking: false });
  await vi.waitFor(() => expect(write).toHaveBeenCalledOnce(), { timeout: 1000 });
  expect(write.mock.calls[0][0][0].page.blocks[0].raw).toBe("[[Fuzzy Existing]] \nid:: 33333333-3333-4333-8333-333333333333");
});
