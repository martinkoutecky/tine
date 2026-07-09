import { afterEach, describe, expect, it } from "vitest";
import { closeInPageFind, inPageFindBlockElement, openInPageFind } from "./inpageFind";
import { focusPane, resetPaneLayoutToSingle, restorePaneLayout } from "./panes";
import { resetStore } from "./store";
import type { PaneSnapshot } from "./router";

const pageSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const journalsSnapshot = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

afterEach(() => {
  closeInPageFind({ restoreFocus: false });
  document.body.innerHTML = "";
  resetStore();
  resetPaneLayoutToSingle(journalsSnapshot());
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
});
