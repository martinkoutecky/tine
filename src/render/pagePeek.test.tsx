import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { installKeybindings } from "../keybindings";
import { clearTransientLayersForTest, dismissTopTransient, registerTransientLayer } from "../transientLayers";

const PEEK_OPEN_MS = 350;
const PEEK_CLOSE_MS = 150;

// GH #40: hovering a [[page]] link shows a read-only hover-peek popup after a
// short dwell. Uses the mock backend (jsdom => mockBackend), so getPage("Tine")
// resolves to a real nested page. No server needed.
beforeAll(async () => {
  await initParser();
});

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  clearTransientLayersForTest();
  vi.useRealTimers();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

function mountAttached(raw: string): { root: HTMLElement; dispose: () => void } {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => (
    <div class="block-content"><AstBody raw={raw} /></div>
  ), host);
  return { root: host, dispose: () => { dispose(); host.remove(); } };
}

async function advance(ms: number) {
  await vi.advanceTimersByTimeAsync(ms);
  await Promise.resolve();
  await Promise.resolve();
}

async function waitFor(fn: () => boolean, timeout = 1000): Promise<boolean> {
  for (let elapsed = 0; elapsed < timeout; elapsed += 25) {
    if (fn()) return true;
    await advance(25);
  }
  return fn();
}

function fireEnter(el: Element) {
  el.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
}
function fireLeave(el: Element) {
  el.dispatchEvent(new MouseEvent("mouseleave", { bubbles: false }));
}

function popup(): HTMLElement | null {
  return document.body.querySelector(".peek-popup");
}

describe("page hover-peek (GH #40)", () => {
  it("renders a portaled RefBlocks tree for a page after dwell", async () => {
    const { root, dispose } = mountAttached("A link to [[Tine]] here");
    try {
      const link = root.querySelector("a.page-ref")!;
      expect(link).toBeTruthy();
      expect(popup()).toBeFalsy();
      expect(document.body.querySelector(".page-ref-preview-line")).toBeFalsy();

      fireEnter(link);
      await advance(PEEK_OPEN_MS);
      const shown = await waitFor(() => !!popup()?.querySelector(".ls-block"));
      expect(shown).toBe(true);

      const card = popup()!;
      expect(document.body.contains(card)).toBe(true);
      expect(root.contains(card)).toBe(false);
      expect(link.contains(card)).toBe(false);
      expect(card.querySelector(".peek-popup-title-name")?.textContent).toContain("Tine");
      expect(card.querySelector(".ls-block")).toBeTruthy();
      expect(card.querySelector(".block-children")).toBeTruthy();
      expect(document.body.querySelector(".page-ref-preview-line")).toBeFalsy();
    } finally {
      dispose();
    }
  });

  it("does not open nested previews from links rendered inside the popup", async () => {
    const { root, dispose } = mountAttached("A link to [[Tine]] here");
    try {
      const link = root.querySelector("a.page-ref")!;
      fireEnter(link);
      await advance(PEEK_OPEN_MS);
      await waitFor(() => !!popup()?.querySelector(".ls-block"));

      const nested = popup()!.querySelector("a.page-ref")!;
      expect(nested).toBeTruthy();
      fireEnter(nested);
      await advance(PEEK_OPEN_MS);
      await waitFor(() => document.body.querySelectorAll(".peek-popup").length === 1);
      expect(document.body.querySelectorAll(".peek-popup")).toHaveLength(1);
    } finally {
      dispose();
    }
  });

  it("uses the close grace to bridge from anchor to popup", async () => {
    const { root, dispose } = mountAttached("go to [[Tine]] now");
    try {
      const link = root.querySelector("a.page-ref")!;
      fireEnter(link);
      await advance(PEEK_OPEN_MS);
      await waitFor(() => !!popup());
      const card = popup()!;

      fireLeave(link);
      await advance(PEEK_CLOSE_MS - 1);
      expect(popup()).toBeTruthy();

      fireEnter(card);
      await advance(PEEK_CLOSE_MS + 1);
      expect(popup()).toBeTruthy();

      fireLeave(card);
      await advance(PEEK_CLOSE_MS);
      expect(popup()).toBeFalsy();
    } finally {
      dispose();
    }
  });

  it("dismisses the popup after anchor leave when the pointer does not enter it", async () => {
    const { root, dispose } = mountAttached("go to [[Tine]] now");
    try {
      const link = root.querySelector("a.page-ref")!;
      fireEnter(link);
      await advance(PEEK_OPEN_MS);
      await waitFor(() => !!popup());

      fireLeave(link);
      await advance(PEEK_CLOSE_MS);
      expect(popup()).toBeFalsy();
    } finally {
      dispose();
    }
  });

  it("keeps the popup open when scrolling inside it, but closes on page scroll", async () => {
    const { root, dispose } = mountAttached("scroll test [[Tine]] here");
    try {
      const link = root.querySelector("a.page-ref")!;
      fireEnter(link);
      await advance(PEEK_OPEN_MS);
      await waitFor(() => !!popup());
      const card = popup()!;

      // Move the pointer onto the popup so the anchor-leave bridge is satisfied
      // and only the scroll listener governs dismissal.
      fireLeave(link);
      fireEnter(card);
      await advance(PEEK_CLOSE_MS + 1);
      expect(popup()).toBeTruthy();

      // Scrolling the popup's own body (or dragging its scrollbar) must NOT close it.
      card.dispatchEvent(new Event("scroll"));
      await advance(PEEK_CLOSE_MS + 1);
      expect(popup()).toBeTruthy();

      // Scrolling the page behind the popup DOES dismiss it.
      document.dispatchEvent(new Event("scroll"));
      await advance(PEEK_CLOSE_MS + 1);
      expect(popup()).toBeFalsy();
    } finally {
      dispose();
    }
  });

  it("shows no popup for a link whose target page does not exist", async () => {
    const { root, dispose } = mountAttached("dangling [[No Such Page 12345]] ref");
    try {
      const link = root.querySelector("a.page-ref")!;
      fireEnter(link);
      await advance(PEEK_OPEN_MS);
      await waitFor(() => !!popup(), 300);
      expect(popup()).toBeFalsy();
    } finally {
      dispose();
    }
  });

  it("opens block-ref peeks only after dwell", async () => {
    const preview = vi.spyOn(backend(), "previewBlock");
    const { root, dispose } = mountAttached("Inline ref ((arch-1)) here");
    try {
      const ref = root.querySelector(".block-ref")!;
      expect(ref).toBeTruthy();
      fireEnter(ref);
      await advance(PEEK_OPEN_MS - 1);
      expect(popup()).toBeFalsy();
      expect(preview).not.toHaveBeenCalled();

      await advance(1);
      const shown = await waitFor(() => !!popup()?.querySelector(".ls-block"));
      expect(shown).toBe(true);
      expect(popup()!.querySelector(".peek-popup-title-name")?.textContent).toContain("Tine");
      expect(popup()!.querySelector(".ls-block")).toBeTruthy();
      expect(preview).toHaveBeenCalledWith("arch-1", 50);
    } finally {
      dispose();
    }
  });

  it("gives a real page peek one shared Escape before a lower transient", async () => {
    const lowerRoot = document.createElement("button");
    document.body.append(lowerRoot);
    let lowerDismissals = 0;
    const unregisterLower = registerTransientLayer({
      id: "peek-lower",
      root: () => lowerRoot,
      dismiss: () => { lowerDismissals += 1; return true; },
    });
    const uninstall = installKeybindings();
    const { root, dispose } = mountAttached("A link to [[Tine]] here");
    try {
      fireEnter(root.querySelector("a.page-ref")!);
      await advance(PEEK_OPEN_MS);
      await waitFor(() => !!popup()?.querySelector(".ls-block"));
      const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
      popup()!.dispatchEvent(event);
      await advance(0);

      expect(event.defaultPrevented).toBe(true);
      expect(popup()).toBeFalsy();
      expect(lowerDismissals).toBe(0);
      document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
      expect(lowerDismissals).toBe(1);
    } finally {
      uninstall();
      unregisterLower();
      dispose();
    }
  });

  it.each([
    ["composing", { composing: true, keyCode: undefined }],
    ["keyCode 229", { composing: false, keyCode: 229 }],
  ] as const)("keeps a real page peek open for %s Escape", async (_name, variant) => {
    const uninstall = installKeybindings();
    const { root, dispose } = mountAttached("A link to [[Tine]] here");
    try {
      fireEnter(root.querySelector("a.page-ref")!);
      await advance(PEEK_OPEN_MS);
      await waitFor(() => !!popup()?.querySelector(".ls-block"));
      const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
      if (variant.composing) Object.defineProperty(event, "isComposing", { value: true });
      if (variant.keyCode != null) Object.defineProperty(event, "keyCode", { value: variant.keyCode });
      popup()!.dispatchEvent(event);
      await advance(PEEK_CLOSE_MS + 1);

      expect(event.defaultPrevented).toBe(false);
      expect(popup()).toBeTruthy();
    } finally {
      uninstall();
      dispose();
    }
  });

  it("uses the same mounted peek rung for shared Back", async () => {
    const { root, dispose } = mountAttached("Inline ref ((arch-1)) here");
    try {
      fireEnter(root.querySelector(".block-ref")!);
      await advance(PEEK_OPEN_MS);
      await waitFor(() => !!popup()?.querySelector(".ls-block"));
      expect(dismissTopTransient("back")).toBe(true);
      await advance(0);
      expect(popup()).toBeFalsy();
    } finally {
      dispose();
    }
  });
});
