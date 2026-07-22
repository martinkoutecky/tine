import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { InPageFind } from "./components/InPageFind";
import {
  clearInPageFindRenderedTextCacheForTests,
  closeInPageFind,
  inPageFindBlockElement,
  inPageFindMatches,
  openInPageFind,
  setInPageFindQuery,
} from "./inpageFind";
import { initParser } from "./render/parse";
import { renderedBlockTextCallCountForTests, resetRenderedBlockTextCallCountForTests } from "./render/renderedText";
import { focusPane, resetPaneLayoutToSingle, restorePaneLayout } from "./panes";
import { resetStore, setDoc } from "./store";
import type { PaneSnapshot } from "./router";

const pageSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const journalsSnapshot = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  setInPageFindQuery("");
  closeInPageFind({ restoreFocus: false });
  document.body.innerHTML = "";
  clearInPageFindRenderedTextCacheForTests();
  resetRenderedBlockTextCallCountForTests();
  resetStore();
  resetPaneLayoutToSingle(journalsSnapshot());
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("in-page find DOM scoping", () => {
  it("resolves duplicate block DOM under the focused pane captured by opening find", () => {
    restorePaneLayout(
      {
        kind: "split",
        dir: "row",
        ratio: 0.5,
        children: [
          { kind: "pane", paneId: "main" },
          { kind: "pane", paneId: "pane-2" },
        ],
      },
      new Map([["main", pageSnapshot("Shared")], ["pane-2", pageSnapshot("Shared")]]),
      "pane-2"
    );
    document.body.innerHTML = `
      <div data-pane-id="main"><div id="main-hit" class="ls-block" data-block-id="shared"></div></div>
      <div data-pane-id="pane-2"><div id="pane-hit" class="ls-block" data-block-id="shared"></div></div>
    `;
    focusPane("pane-2");

    openInPageFind();

    expect(inPageFindBlockElement("shared")).toBe(document.getElementById("pane-hit"));
  });

  it("debounces rapid query input to one in-page find scan", async () => {
    vi.useFakeTimers();
    const byId = Object.fromEntries(
      Array.from({ length: 64 }, (_, i) => [
        `block-${i}`,
        { id: `block-${i}`, raw: `needle target ${i}`, collapsed: false, parent: null, page: "Pane", children: [] },
      ])
    );
    setDoc({
      loaded: true,
      feed: ["Pane"],
      pages: [
        {
          name: "Pane",
          kind: "page",
          title: "Pane",
          preBlock: null,
          roots: Object.keys(byId),
          format: "md",
          readOnly: false,
          guide: false,
        },
      ],
      byId,
    });
    resetPaneLayoutToSingle(pageSnapshot("Pane"));
    focusPane("main");
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <InPageFind />, host);
    try {
      openInPageFind();
      await Promise.resolve();
      const input = host.querySelector(".inpage-find-input") as HTMLInputElement;
      expect(input).toBeTruthy();

      for (const q of ["n", "ne", "nee", "need"]) {
        input.value = q;
        input.dispatchEvent(new InputEvent("input", { bubbles: true }));
        await vi.advanceTimersByTimeAsync(25);
      }

      expect(renderedBlockTextCallCountForTests()).toBe(0);
      await vi.advanceTimersByTimeAsync(110);
      expect(inPageFindMatches()).toHaveLength(64);
      expect(renderedBlockTextCallCountForTests()).toBe(64);
    } finally {
      dispose();
      host.remove();
    }
  });

  it("keeps the caret collapsed while typing and selects all only on an explicit refocus (GH #224)", async () => {
    vi.useFakeTimers();
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <InPageFind />, host);
    try {
      openInPageFind();
      await Promise.resolve();
      const input = host.querySelector(".inpage-find-input") as HTMLInputElement;
      expect(input).toBeTruthy();

      input.value = "a";
      input.setSelectionRange(1, 1);
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await vi.advanceTimersByTimeAsync(110);
      await Promise.resolve();

      expect(input.value).toBe("a");
      expect([input.selectionStart, input.selectionEnd]).toEqual([1, 1]);

      // Repeating Ctrl+F is a distinct semantic request: focus the existing
      // query and select it so the user can replace it in one keystroke.
      openInPageFind();
      await Promise.resolve();
      expect([input.selectionStart, input.selectionEnd]).toEqual([0, 1]);
    } finally {
      dispose();
      host.remove();
    }
  });
});
