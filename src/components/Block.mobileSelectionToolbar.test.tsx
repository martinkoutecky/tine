import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { loadSingle, pageByName, resetStore } from "../store";
import type { BlockDto } from "../types";

const platform = vi.hoisted(() => ({ mobile: true }));
vi.mock("../nativeChrome", () => ({
  get isMobilePlatform() {
    return platform.mobile;
  },
}));

beforeAll(() => initParser());

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
  platform.mobile = true;
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

async function mountEditor(mobile: boolean) {
  platform.mobile = mobile;
  const { Block } = await import("./Block");
  const block: BlockDto = {
    id: "mobile-selection-toolbar",
    raw: "alpha selected omega",
    collapsed: false,
    children: [],
  };
  loadSingle({ name: "Mobile selection", kind: "page", title: "Mobile selection", pre_block: null, blocks: [block] });
  startEditing(block.id, 0);
  const mounted = mount(() => (
    <For each={pageByName("Mobile selection")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ));
  const textarea = mounted.root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
  textarea.focus();
  return { ...mounted, textarea };
}

async function mountSelectedEditor(mobile: boolean) {
  const mounted = await mountEditor(mobile);
  const { textarea } = mounted;
  textarea.setSelectionRange(6, 14);
  textarea.dispatchEvent(new Event("select", { bubbles: true }));
  await vi.waitFor(() => expect(mounted.root.querySelector(".sel-toolbar")).not.toBeNull());
  return { ...mounted, textarea, toolbar: mounted.root.querySelector<HTMLElement>(".sel-toolbar")! };
}

describe("selected-text toolbar platform ownership (GH #375)", () => {
  it("tracks native selectionchange without a select or mouseup event", async () => {
    const mounted = await mountEditor(true);
    try {
      let start = 6;
      let end = 14;
      // Native WebView selection updates the DOM range without the desktop
      // select/mouseup events. Avoid jsdom's synthetic setSelectionRange events.
      Object.defineProperty(mounted.textarea, "selectionStart", { configurable: true, get: () => start });
      Object.defineProperty(mounted.textarea, "selectionEnd", { configurable: true, get: () => end });
      document.dispatchEvent(new Event("selectionchange"));
      expect(mounted.root.querySelector("[data-mobile-selection-toolbar]")).not.toBeNull();
      start = end;
      mounted.textarea.dispatchEvent(new Event("selectionchange"));
      expect(mounted.root.querySelector("[data-mobile-selection-toolbar]")).toBeNull();
    } finally {
      mounted.dispose();
    }
  });

  it("ignores selection changes owned by another focused control", async () => {
    const mounted = await mountEditor(true);
    const other = document.createElement("textarea");
    document.body.append(other);
    try {
      other.focus();
      Object.defineProperty(mounted.textarea, "selectionStart", { configurable: true, get: () => 6 });
      Object.defineProperty(mounted.textarea, "selectionEnd", { configurable: true, get: () => 14 });
      document.dispatchEvent(new Event("selectionchange"));
      expect(mounted.root.querySelector("[data-mobile-selection-toolbar]")).toBeNull();
    } finally {
      other.remove();
      mounted.dispose();
    }
  });

  it("marks the mobile formatting surface for keyboard-dock positioning", async () => {
    const mounted = await mountSelectedEditor(true);
    try {
      expect(mounted.toolbar.classList.contains("sel-toolbar-mobile")).toBe(true);
      expect(mounted.toolbar.dataset.mobileSelectionToolbar).toBe("");
    } finally {
      mounted.dispose();
    }
  });

  it("keeps desktop selection formatting editor-anchored", async () => {
    const mounted = await mountSelectedEditor(false);
    try {
      expect(mounted.toolbar.classList.contains("sel-toolbar-mobile")).toBe(false);
      expect(mounted.toolbar.hasAttribute("data-mobile-selection-toolbar")).toBe(false);
    } finally {
      mounted.dispose();
    }
  });
});
