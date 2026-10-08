import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { pluginManager } from "../plugins/manager";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

function node(id: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw: id, collapsed: false, parent, page: "Decorations", children };
}

function page(roots: string[]): FeedPage {
  return {
    name: "Decorations",
    kind: "page",
    title: "Decorations",
    preBlock: null,
    roots,
    format: "md",
    readOnly: false,
    guide: false,
  };
}

// Master ce9a796fb: `thread-lines` only decorates the children container below a
// row, so a leaf (most blocks of a large flat page) must not subscribe to plugin
// installation/settings changes at all.
describe("plugin thread-lines decoration host", () => {
  it("does not subscribe leaf blocks, then decorates normally after a child is added", () => {
    const [enabled, setEnabled] = createSignal(true);
    const [display, setDisplay] = createSignal("active");
    const [intensity, setIntensity] = createSignal("standard");
    const hasDecoration = vi.spyOn(pluginManager, "hasDeclarativeDecoration").mockImplementation(
      (kind) => kind === "thread-lines" && enabled(),
    );
    const decorationSetting = vi.spyOn(pluginManager, "declarativeDecorationSetting").mockImplementation(
      (kind, key) => {
        if (kind !== "thread-lines") return undefined;
        if (key === "display") return display();
        if (key === "intensity") return intensity();
        return undefined;
      },
    );

    setDoc({
      byId: { root: node("root", null) },
      pages: [page(["root"])],
      feed: ["Decorations"],
      loaded: true,
    });

    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <Block id="root" />, host);
    try {
      const root = host.querySelector<HTMLElement>('[data-block-id="root"]')!;
      expect(hasDecoration).not.toHaveBeenCalled();
      expect(decorationSetting).not.toHaveBeenCalled();
      expect(root.classList.contains("plugin-thread-lines")).toBe(false);

      // The host becomes eligible reactively. Its new leaf child stays cheap.
      setDoc("byId", "child", node("child", "root"));
      setDoc("byId", "root", "children", ["child"]);

      expect(hasDecoration).toHaveBeenCalledTimes(1);
      expect(decorationSetting).toHaveBeenCalledTimes(2);
      expect(root.classList.contains("plugin-thread-lines")).toBe(true);
      expect(root.classList.contains("plugin-thread-lines-active")).toBe(true);
      expect(root.classList.contains("plugin-thread-lines-standard")).toBe(true);

      // The parent remains subscribed to the real plugin state after it becomes
      // a decoration host, rather than merely receiving a one-time class list.
      setEnabled(false);
      setDisplay("hidden");
      setIntensity("muted");
      expect(root.classList.contains("plugin-thread-lines")).toBe(false);
      expect(root.classList.contains("plugin-thread-lines-active")).toBe(false);
      expect(root.classList.contains("plugin-thread-lines-standard")).toBe(false);
    } finally {
      dispose();
    }
  });
});
