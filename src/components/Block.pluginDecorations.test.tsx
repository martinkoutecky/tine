import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { pluginManager } from "../plugins/manager";
import { Block } from "./Block";
import { rowDecorationStyle, threadLineSettings } from "./block/rowChrome";

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
  it("bounds host styles, keeps theme defaults, and repeats hierarchy colors", () => {
    for (const thickness of [0, 1, 4, 10]) {
      const thread = threadLineSettings(true, key => key === "thickness" ? thickness : undefined);
      expect(rowDecorationStyle(thread)["--thread-width"]).toBe(`${thickness}px`);
      expect(rowDecorationStyle(thread)["--thread-color"]).toBe("var(--ls-link-text-color)");
    }
    for (const thickness of [-1, 11, Number.NaN, "4px"]) {
      const thread = threadLineSettings(true, key => key === "thickness" ? thickness : "url(x)");
      expect(rowDecorationStyle(thread)["--thread-width"]).toBe("1px");
      expect(rowDecorationStyle(thread)["--thread-color"]).toBe("var(--ls-link-text-color)");
    }
    const levels = threadLineSettings(true, key => key === "color-by-level" ? true : undefined);
    expect(rowDecorationStyle(levels, 0)["--thread-active-color"]).toBe(rowDecorationStyle(levels, 4)["--thread-active-color"]);
    expect(rowDecorationStyle(levels, 1)["--thread-active-color"]).not.toBe(rowDecorationStyle(levels, 0)["--thread-active-color"]);
  });
  it("does not subscribe leaf blocks, then decorates normally after a child is added", () => {
    const [enabled, setEnabled] = createSignal(true);
    const [display, setDisplay] = createSignal("active");
    const [intensity, setIntensity] = createSignal("standard");
    const [thickness, setThickness] = createSignal(4);
    const hasDecoration = vi.spyOn(pluginManager, "hasDeclarativeDecoration").mockImplementation(
      (kind) => kind === "thread-lines" && enabled(),
    );
    const decorationSetting = vi.spyOn(pluginManager, "declarativeDecorationSetting").mockImplementation(
      (kind, key) => {
        if (kind !== "thread-lines") return undefined;
        if (key === "display") return display();
        if (key === "intensity") return intensity();
        if (key === "thickness") return thickness();
        if (key === "color") return "#e05070";
        if (key === "color-by-level") return true;
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
      expect(decorationSetting).toHaveBeenCalled();
      expect(root.classList.contains("plugin-thread-lines")).toBe(true);
      expect(root.classList.contains("plugin-thread-lines-active")).toBe(true);
      expect(root.classList.contains("plugin-thread-lines-standard")).toBe(true);
      expect(root.classList.contains("plugin-thread-lines-levels")).toBe(true);
      expect(root.style.getPropertyValue("--thread-width")).toBe("4px");
      expect(root.style.getPropertyValue("--thread-color")).toBe("#e05070");
      setThickness(0);
      expect(root.classList.contains("plugin-thread-lines-hidden")).toBe(true);

      // The parent remains subscribed to the real plugin state after it becomes
      // a decoration host, rather than merely receiving a one-time class list.
      setEnabled(false);
      setDisplay("hidden");
      setIntensity("muted");
      expect(root.classList.contains("plugin-thread-lines")).toBe(false);
      expect(root.classList.contains("plugin-thread-lines-active")).toBe(false);
      expect(root.classList.contains("plugin-thread-lines-standard")).toBe(false);
      expect(root.classList.contains("plugin-thread-lines-levels")).toBe(false);
      expect(root.classList.contains("plugin-thread-lines-hidden")).toBe(false);
      expect(root.style.getPropertyValue("--thread-width")).toBe("");
    } finally {
      dispose();
    }
  });
});
