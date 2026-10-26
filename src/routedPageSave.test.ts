import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { resetStore, setRaw } from "./document";
import { bindTestHost, submittedPages } from "./document/host/wiring.test.support";
import { loadRoutedPage } from "./document/workingSet";

afterEach(() => { resetStore(); vi.restoreAllMocks(); });

it("saves a page routed directly after startup, without first opening the journal feed", async () => {
  resetStore();
  const write = vi.spyOn(backend(), "pageSubmit");
  loadRoutedPage({ name: "Routed", kind: "page", title: "Routed", id: "pages/Routed.md",
    rev: "base-rev", pre_block: null, blocks: [
      { id: "block", raw: "id:: 33333333-3333-4333-8333-333333333333", collapsed: false, children: [] },
    ] });
  await bindTestHost();
  setRaw("block", "[[Fuzzy Existing]] \nid:: 33333333-3333-4333-8333-333333333333", { timetracking: false });
  await vi.waitFor(() => expect(write).toHaveBeenCalledOnce(), { timeout: 1000 });
  expect(submittedPages(write)[0].blocks[0].raw).toBe("[[Fuzzy Existing]] \nid:: 33333333-3333-4333-8333-333333333333");
});
