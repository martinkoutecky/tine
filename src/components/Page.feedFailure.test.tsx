import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { PageView } from "./Page";
import { resetTabsToJournals } from "../router";
import { setGraphMeta } from "../graphSession";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  document.body.innerHTML = "";
  resetTabsToJournals();
});
function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

describe("journal feed read failures (GH #385, master d6024bac3aad)", () => {
  it("surfaces an initial feed read failure instead of claiming the graph has no journals", async () => {
    vi.spyOn(backend(), "journalFeedPage").mockRejectedValue(new Error("iCloud journal read failed"));
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("iCloud journal read failed"));
      expect(mounted.root.textContent).toContain("Couldn't open this page");
      expect(mounted.root.textContent).not.toContain("No journal entries found");
    } finally {
      mounted.dispose();
    }
  });
});
