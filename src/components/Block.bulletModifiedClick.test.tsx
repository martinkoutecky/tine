// GH #456: the outline bullet read only Shift, so Ctrl/Cmd+click and Alt+click
// on it fell through to the plain zoom and the modifier did nothing — while the
// very same modifiers on a [[page ref]] already opened a background tab or the
// other pane. The bullet now goes through the one shared decision in
// linkGesture.ts, so all four destinations agree across the app.

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { layoutPaneIds, paneRouter, resetPaneLayoutToSingle } from "../panes";
import { rightSidebar, setRightSidebar } from "../ui";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

// The user is looking at the page whose bullet they click, which is what makes
// "opened somewhere else" distinguishable from "zoomed here".
const bulletsSnapshot = () => ({
  tabs: [{
    history: [{ kind: "page" as const, name: "Bullets", pageKind: "page" as const }],
    pos: 0,
    pinned: false,
  }],
  activeIndex: 0,
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setRightSidebar([]);
  resetPaneLayoutToSingle(bulletsSnapshot());
  document.body.innerHTML = "";
});

const page = (): PageDto => ({
  name: "Bullets",
  kind: "page",
  title: "Bullets",
  pre_block: null,
  blocks: [{ id: "A", raw: "target block", collapsed: false, children: [] } as BlockDto],
});

function mountBullet(): { bullet: HTMLElement; dispose: () => void } {
  loadSingle(page());
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(
    () => <For each={pageByName("Bullets")?.roots ?? []}>{(id) => <Block id={id} />}</For>,
    host,
  );
  return {
    bullet: host.querySelector<HTMLElement>('[data-block-id="A"] > .block-main .bullet-container')!,
    dispose,
  };
}

function click(el: HTMLElement, init: MouseEventInit) {
  el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ...init }));
}

function mouseDownPrevented(el: HTMLElement, init: MouseEventInit): boolean {
  const e = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, ...init });
  el.dispatchEvent(e);
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true })); // end any drag
  return e.defaultPrevented;
}

describe("outline bullet modified clicks (GH #456)", () => {
  it("opens a background tab on Ctrl/Cmd+click instead of zooming in place", async () => {
    for (const mod of [{ ctrlKey: true }, { metaKey: true }]) {
      resetPaneLayoutToSingle(bulletsSnapshot());
      const { bullet, dispose } = mountBullet();
      try {
        const before = paneRouter("main").tabs().length;
        click(bullet, mod);
        // og opens block destinations only once the target id has reached disk.
        await vi.waitFor(() => expect(paneRouter("main").tabs().length).toBe(before + 1));
        const tabs = paneRouter("main").tabs();
        expect(tabs.some((t) => {
          const r = t.history[t.pos];
          return r.kind === "page" && r.name === "Bullets" && !!r.block;
        })).toBe(true);
        // Background: the pane the user was looking at did not move.
        expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Bullets" });
        expect((paneRouter("main").route() as { block?: string }).block).toBeUndefined();
      } finally {
        dispose();
        resetStore();
        document.body.innerHTML = "";
      }
    }
  });

  it("opens the other pane on Alt+click", async () => {
    const { bullet, dispose } = mountBullet();
    try {
      expect(layoutPaneIds()).toEqual(["main"]);
      click(bullet, { altKey: true });
      await vi.waitFor(() => expect(layoutPaneIds().length).toBe(2));
    } finally {
      dispose();
    }
  });

  it("suppresses the browser defaults those gestures replace (GH #207)", () => {
    const { bullet, dispose } = mountBullet();
    try {
      // Shift-range selection and middle-button autoscroll / PRIMARY-paste.
      expect(mouseDownPrevented(bullet, { shiftKey: true })).toBe(true);
      expect(mouseDownPrevented(bullet, { button: 1 })).toBe(true);
      expect(mouseDownPrevented(bullet, {})).toBe(false);
    } finally {
      dispose();
    }
  });

  it("keeps Shift+click on the sidebar and a plain click on the in-place zoom", async () => {
    const { bullet, dispose } = mountBullet();
    try {
      click(bullet, { shiftKey: true });
      await vi.waitFor(() => expect(rightSidebar()).toHaveLength(1));
      expect(rightSidebar()[0]).toMatchObject({ kind: "block", page: "Bullets" });

      const before = paneRouter("main").tabs().length;
      click(bullet, {});
      expect(paneRouter("main").tabs().length).toBe(before);
      await vi.waitFor(() => expect((paneRouter("main").route() as { block?: string }).block).toBeTruthy());
    } finally {
      dispose();
    }
  });
});
