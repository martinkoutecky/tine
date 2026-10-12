import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { editingId, endEdit } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { setToasts, toasts } from "../toasts";
import * as router from "../router";
import type { BlockDto, PageDto, RefGroup } from "../types";
import { Block } from "./Block";
import { installKeybindings } from "../keybindings";

let keys = () => {};

// GH #641: Move Block Up/Down on the embedded ROOT moves the source block among
// its own siblings (OG parity, kept: Martin 2026-10-06), and nothing visible
// changes in the embed. Tine says where it went: a short notice naming the page,
// with a way to open the block there. Moves the embed itself shows (a nested row
// among its siblings) and ordinary outline moves stay silent.

class AllNearObserver implements IntersectionObserver {
  readonly root = null;
  readonly rootMargin = "0px";
  readonly thresholds = [0];
  constructor(private readonly callback: IntersectionObserverCallback) {}
  disconnect(): void {}
  takeRecords(): IntersectionObserverEntry[] { return []; }
  unobserve(): void {}
  observe(target: Element): void {
    this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this);
  }
}

beforeAll(async () => {
  await initParser();
});

beforeEach(() => {
  vi.stubGlobal("IntersectionObserver", AllNearObserver);
  setToasts([]);
  keys = installKeybindings();
});

afterEach(() => {
  keys();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  endEdit("page-navigation");
  resetStore();
  setToasts([]);
  document.body.innerHTML = "";
});

const leaf = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });

function loadFixture() {
  const target = leaf("target", "embedded root", [leaf("c1", "child one"), leaf("c2", "child two")]);
  const src: PageDto = { name: "Src", kind: "page", title: "Src", pre_block: null, blocks: [leaf("first", "first on src"), target] };
  const host: PageDto = { name: "HostPage", kind: "page", title: "HostPage", pre_block: null,
    blocks: [leaf("above", "plain above"), leaf("host", "{{embed ((target))}}"), leaf("below", "plain below")] };
  const group: RefGroup = { page: "Src", kind: "page", blocks: [{ ...target, children: [] }] };
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map((id) => (id === "target" ? group : null)));
  loadSingle(src);
  loadSingle(host);
}

function mount() {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => (
    <For each={pageByName("HostPage")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  return { root, dispose };
}

async function editAndMoveUp(root: HTMLElement, selector: string, id: string) {
  const content = await vi.waitFor(() => {
    const el = root.querySelector<HTMLElement>(`${selector} > .block-main .block-content`);
    expect(el).not.toBeNull();
    return el!;
  });
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  await vi.waitFor(() => expect(editingId()).toBe(id));
  const editor = await vi.waitFor(() => {
    const el = root.querySelector<HTMLTextAreaElement>(`${selector} textarea.block-editor`);
    expect(el).not.toBeNull();
    return el!;
  });
  editor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
}

describe("move notice for moves the view cannot show (GH #641)", () => {
  it("Move Up on the embedded root names the page it moved in and opens the block there", async () => {
    loadFixture();
    const open = vi.spyOn(router, "openPageAtBlock").mockImplementation(() => {});
    const { root, dispose } = mount();
    try {
      await editAndMoveUp(root, `.embed-block [data-block-id="target"]`, "target");
      await vi.waitFor(() => expect(pageByName("Src")!.roots).toEqual(["target", "first"]));
      const notice = await vi.waitFor(() => {
        const toast = toasts().find((t) => t.message.includes("Src"));
        expect(toast).toBeDefined();
        return toast!;
      });
      expect(notice.kind).toBe("info");
      expect(notice.sticky).toBeFalsy();
      notice.action!.run();
      expect(open).toHaveBeenCalledWith(expect.objectContaining({ name: "Src", pageKind: "page", block: "target" }));
    } finally {
      dispose();
    }
  });

  it("a move the embed shows (a nested row among its siblings) stays silent", async () => {
    loadFixture();
    const { root, dispose } = mount();
    try {
      await editAndMoveUp(root, `.embed-block [data-block-id="c2"]`, "c2");
      await vi.waitFor(() => expect(pageByName("Src") && root.querySelector(".embed-block")).toBeTruthy());
      await vi.waitFor(() => expect(["c2", "c1"]).toEqual(
        [...root.querySelectorAll('.embed-block [data-block-id="target"] .block-children > [data-block-id]')].map((el) => el.getAttribute("data-block-id")),
      ));
      await new Promise((r) => setTimeout(r, 20));
      expect(toasts()).toEqual([]);
    } finally {
      dispose();
    }
  });

  it("an ordinary outline move stays silent", async () => {
    loadFixture();
    const { root, dispose } = mount();
    try {
      await editAndMoveUp(root, `[data-block-id="below"]`, "below");
      await vi.waitFor(() => expect(pageByName("HostPage")!.roots).toEqual(["above", "below", "host"]));
      await new Promise((r) => setTimeout(r, 20));
      expect(toasts()).toEqual([]);
    } finally {
      dispose();
    }
  });
});
