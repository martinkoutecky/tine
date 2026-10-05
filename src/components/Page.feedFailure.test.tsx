import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { PageView } from "./Page";
import { resetTabsToJournals } from "../router";
import { setGraphMeta } from "../graphSession";
import { toasts, setToasts } from "../toasts";
import { localDayKey } from "../journal";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  document.body.innerHTML = "";
  resetTabsToJournals();
  setToasts([]);
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

  // og C5 P2 (I-22): a journal the backend skipped is named, never silently absent.
  it("reports journals the backend skipped as unreadable", async () => {
    vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true, as_of_day: localDayKey(),
      unreadable: ["journals/2026_01_02.md: stream did not contain valid UTF-8"],
    });
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(toasts()).toEqual([
        expect.objectContaining({ kind: "error", message: expect.stringContaining("couldn't be read") }),
      ]));
    } finally {
      mounted.dispose();
    }
  });
});
