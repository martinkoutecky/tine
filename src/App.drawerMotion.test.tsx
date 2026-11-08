import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { dismissDrawerAndRestore } from "./components/MobileDrawerShell";
import { activeDrawer, setLeftSidebarOpen, setRightSidebarOpen } from "./ui";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.(); dispose = undefined;
  setLeftSidebarOpen(false); setRightSidebarOpen(false);
  vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks();
  delete (Element.prototype as Partial<Element>).animate;
  document.body.replaceChildren();
});

// Exercise the production App ownership seam too: an outer Show around either
// sidebar used to destroy the surface before a closing animation could run.
describe("GH #501 production App drawer lifetime", () => {
  for (const side of ["left", "right"] as const) {
    it(`${side}: animates opening and retains an inaccessible closing surface until motion finishes`, async () => {
      vi.useFakeTimers();
      vi.stubGlobal("matchMedia", (query: string) => ({ matches: query === "(max-width: 639px)", addEventListener() {}, removeEventListener() {} }));
      const animate = vi.fn(() => ({ cancel() {}, onfinish: null }) as unknown as Animation);
      Object.defineProperty(Element.prototype, "animate", { configurable: true, value: animate });
      const root = document.createElement("div"); document.body.append(root);
      dispose = render(() => <App />, root);
      setLeftSidebarOpen(false); setRightSidebarOpen(false);
      await Promise.resolve(); await Promise.resolve();
      animate.mockClear();
      (side === "left" ? setLeftSidebarOpen : setRightSidebarOpen)(true);
      const panel = root.querySelector<HTMLElement>(`.${side}-sidebar`)!;
      expect(panel).not.toBeNull();
      expect(animate).toHaveBeenCalled();
      vi.advanceTimersByTime(450);
      dismissDrawerAndRestore("escape");
      expect(activeDrawer()).toBeNull();
      expect(panel.isConnected).toBe(true);
      expect(panel.inert || panel.hasAttribute("inert")).toBe(true);
      expect(panel.getAttribute("aria-hidden")).toBe("true");
      vi.advanceTimersByTime(450);
      expect(panel.isConnected).toBe(false);
    });
  }
});
