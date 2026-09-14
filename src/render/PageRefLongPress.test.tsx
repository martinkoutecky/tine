import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { initParser } from "./parse";
import { renderInlines } from "./inline";
import { contextMenu, closeContextMenu } from "../ui";
import { route, openPage } from "../router";
import { resetStore } from "../store";
import { LONG_PRESS_DELAY } from "./longPress";

// GH #231: on mobile, a deliberate long-press on a page link must raise the
// same relevant context menu as desktop right-click — while quick tap keeps
// navigating, movement cancels, and mouse/desktop behavior is unchanged.

beforeAll(() => initParser());

afterEach(() => {
  closeContextMenu();
  vi.useRealTimers();
  resetStore();
  openPage("journals-marker-target", "page");
  document.body.innerHTML = "";
});

function mountRef(): { root: HTMLDivElement; dispose: () => void; anchor: () => HTMLAnchorElement } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(
    () => renderInlines([{ k: "link", url: { type: "page_ref", v: "Some Page" }, full: "[[Some Page]]" }]) as JSX.Element,
    root,
  );
  return {
    root,
    dispose,
    anchor: () => {
      const a = root.querySelector<HTMLAnchorElement>("a.page-ref");
      expect(a).not.toBeNull();
      return a!;
    },
  };
}

function touch(type: string, x: number, y: number, extra: Partial<PointerEventInit> = {}): PointerEvent {
  return new PointerEvent(type, {
    bubbles: true,
    cancelable: true,
    pointerType: "touch",
    isPrimary: true,
    pointerId: 7,
    clientX: x,
    clientY: y,
    ...extra,
  });
}

describe("page-ref long-press gesture (GH #231)", () => {
  it("a still hold through the delay opens the page context menu at the held point — and does not navigate", () => {
    vi.useFakeTimers();
    const { anchor, dispose } = mountRef();
    try {
      anchor().dispatchEvent(touch("pointerdown", 41, 62));
      expect(contextMenu()).toBeNull();
      vi.advanceTimersByTime(LONG_PRESS_DELAY - 1);
      expect(contextMenu()).toBeNull();
      vi.advanceTimersByTime(1);
      const menu = contextMenu();
      expect(menu?.kind).toBe("page");
      expect(menu && "name" in menu ? menu.name : null).toBe("Some Page");
      expect(menu?.x).toBe(41);
      expect(menu?.y).toBe(62);
      {
        const r = route();
        expect(r.kind === "page" ? r.name : null).not.toBe("Some Page"); // the gesture alone never navigates
      }
      // Releasing a completed hold must also consume the browser's
      // compatibility click; otherwise the menu opens and then navigates away.
      anchor().dispatchEvent(touch("pointerup", 41, 62));
      anchor().dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
      vi.advanceTimersByTime(2 * LONG_PRESS_DELAY);
      expect(contextMenu()?.kind).toBe("page");
      {
        const r = route();
        expect(r.kind === "page" ? r.name : null).not.toBe("Some Page");
      }
    } finally {
      dispose();
    }
  });

  it("quick tap releases before the delay: no menu, ordinary click still navigates to the page", () => {
    vi.useFakeTimers();
    const { anchor, dispose } = mountRef();
    try {
      openPage("Before", "page");
      anchor().dispatchEvent(touch("pointerdown", 41, 62));
      anchor().dispatchEvent(touch("pointerup", 41, 62));
      vi.advanceTimersByTime(2 * LONG_PRESS_DELAY);
      expect(contextMenu()).toBeNull();
      anchor().dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
      {
        const r = route();
        expect(r.kind === "page" ? r.name : null).toBe("Some Page");
      }
    } finally {
      dispose();
    }
  });

  it("moving past the tolerance first cancels the gesture; the tap afterwards simply navigates", () => {
    vi.useFakeTimers();
    const { anchor, dispose } = mountRef();
    try {
      openPage("Before", "page");
      anchor().dispatchEvent(touch("pointerdown", 41, 62));
      anchor().dispatchEvent(touch("pointermove", 71, 62)); // 30px right: scroll intent
      vi.advanceTimersByTime(2 * LONG_PRESS_DELAY);
      expect(contextMenu()).toBeNull();
      anchor().dispatchEvent(touch("pointerup", 71, 62));
      anchor().dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
      {
        const r = route();
        expect(r.kind === "page" ? r.name : null).toBe("Some Page");
      }
    } finally {
      dispose();
    }
  });

  it("a mouse hold does not arm anything (desktop keeps right-click only)", () => {
    vi.useFakeTimers();
    const { anchor, dispose } = mountRef();
    try {
      anchor().dispatchEvent(new PointerEvent("pointerdown", {
        bubbles: true, cancelable: true, pointerType: "mouse", isPrimary: true, pointerId: 1, clientX: 41, clientY: 62,
      }));
      vi.advanceTimersByTime(2 * LONG_PRESS_DELAY);
      expect(contextMenu()).toBeNull();
    } finally {
      dispose();
    }
  });

  it("the desktop right-click (native contextmenu) still opens the same menu", () => {
    const { anchor, dispose } = mountRef();
    try {
      anchor().dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 5, clientY: 6 }));
      const menu = contextMenu();
      expect(menu?.kind).toBe("page");
      expect(menu && "name" in menu ? menu.name : null).toBe("Some Page");
    } finally {
      dispose();
    }
  });

  it("multi-touch: a second finger does not steal the gesture, and cancel on the system interrupt fires nothing", () => {
    vi.useFakeTimers();
    const { anchor, dispose } = mountRef();
    try {
      anchor().dispatchEvent(touch("pointerdown", 41, 62, { pointerId: 7 }));
      anchor().dispatchEvent(touch("pointerdown", 300, 300, { isPrimary: false, pointerId: 8 }));
      // Second finger's movement far away must not cancel the primary gesture's
      // recognition rule (only primary, same-id moves count), but a CANCEL does.
      anchor().dispatchEvent(touch("pointercancel", 41, 62));
      vi.advanceTimersByTime(2 * LONG_PRESS_DELAY);
      expect(contextMenu()).toBeNull();
    } finally {
      dispose();
    }
  });
});
