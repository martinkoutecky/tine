import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { HelpPopup } from "./HelpShortcuts";
import { InPageFind } from "./InPageFind";
import { Lightbox } from "./Toasts";
import { installKeybindings } from "../keybindings";
import { closeInPageFind, openInPageFind } from "../inpageFind";
import { closeHelpPopup, lightbox, setLightbox, toggleHelpPopup } from "../ui";
import {
  clearTransientLayersForTest,
  dismissTopTransient,
  registerTransientLayer,
  topTransientLayer,
} from "../transientLayers";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function escape() {
  return new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
}

function mountedHost() {
  const host = document.createElement("div");
  document.body.append(host);
  return host;
}

function registryListeners(calls: unknown[][]) {
  return calls.filter(([type, _listener, capture]) =>
    (type === "focusin" || type === "pointerdown") && capture === true,
  ) as ["focusin" | "pointerdown", EventListener, true][];
}

function expectRemoved(removeSpy: ReturnType<typeof vi.spyOn>, listeners: ["focusin" | "pointerdown", EventListener, true][]) {
  expect(listeners.map(([type]) => type).sort()).toEqual(["focusin", "pointerdown"]);
  for (const [type, listener, capture] of listeners) {
    expect(removeSpy).toHaveBeenCalledWith(type, listener, capture);
  }
}

afterEach(() => {
  closeInPageFind({ restoreFocus: false });
  closeHelpPopup();
  setLightbox(null);
  // All owned registrations must already have been explicitly unregistered or
  // component-cleaned; this is only the test assertion reset.
  clearTransientLayersForTest();
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("GH #161 P1D1-C registry identity and listener lifecycle", () => {
  it("reactivates real Find by focus, then routes window Escape through the global dispatcher without closing newer Help", async () => {
    const host = mountedHost();
    const dispose = render(() => <><InPageFind /><HelpPopup /></>, host);
    const uninstall = installKeybindings();
    try {
      openInPageFind();
      await tick();
      toggleHelpPopup();
      await tick();
      const find = host.querySelector<HTMLInputElement>(".inpage-find-input")!;
      find.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));

      expect(host.querySelector(".inpage-find-bar")).not.toBeNull();
      expect(host.querySelector(".help-menu")).not.toBeNull();
      expect(topTransientLayer()?.id).toBe("in-page-find");

      const event = escape();
      window.dispatchEvent(event);
      await tick();
      expect(event.defaultPrevented).toBe(true);
      expect(host.querySelector(".inpage-find-bar")).toBeNull();
      expect(host.querySelector(".help-menu")).not.toBeNull();
    } finally {
      closeInPageFind({ restoreFocus: false });
      closeHelpPopup();
      await tick();
      uninstall();
      dispose();
    }
  });

  it("reactivates real Help by an inside pointer, then routes window Escape through the global dispatcher without closing newer Find", async () => {
    const host = mountedHost();
    const dispose = render(() => <><InPageFind /><HelpPopup /></>, host);
    const uninstall = installKeybindings();
    try {
      toggleHelpPopup();
      await tick();
      openInPageFind();
      await tick();
      // The registered root is the Help corner itself; dispatch inside that
      // root so Help's window-capture outside-pointer closer is not involved.
      const help = host.querySelector<HTMLElement>(".help-corner")!;
      help.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));

      expect(host.querySelector(".help-menu")).not.toBeNull();
      expect(host.querySelector(".inpage-find-bar")).not.toBeNull();
      expect(topTransientLayer()?.id).toBe("help");

      const event = escape();
      window.dispatchEvent(event);
      await tick();
      expect(event.defaultPrevented).toBe(true);
      expect(host.querySelector(".help-menu")).toBeNull();
      expect(host.querySelector(".inpage-find-bar")).not.toBeNull();
    } finally {
      closeInPageFind({ restoreFocus: false });
      closeHelpPopup();
      await tick();
      uninstall();
      dispose();
    }
  });

  it("removes the exact document listeners for ordinary close and component disposal, so reconnected old roots cannot reactivate", async () => {
    const host = mountedHost();
    const addSpy = vi.spyOn(document, "addEventListener");
    const removeSpy = vi.spyOn(document, "removeEventListener");
    const dispose = render(() => <><InPageFind /><HelpPopup /></>, host);
    const uninstall = installKeybindings();
    let unregisterFresh = () => {};
    try {
      const beforeFind = addSpy.mock.calls.length;
      openInPageFind();
      await tick();
      const oldFind = host.querySelector<HTMLElement>(".inpage-find-bar")!;
      const findListeners = registryListeners(addSpy.mock.calls.slice(beforeFind));
      closeInPageFind({ restoreFocus: false });
      await tick();
      expectRemoved(removeSpy, findListeners);

      const beforeHelp = addSpy.mock.calls.length;
      toggleHelpPopup();
      await tick();
      const oldHelp = host.querySelector<HTMLElement>(".help-corner")!;
      const helpListeners = registryListeners(addSpy.mock.calls.slice(beforeHelp));
      dispose();
      expectRemoved(removeSpy, helpListeners);

      // Keep the old DOM roots observable after their ordinary/component
      // cleanup.  Their former handlers must not re-register or steal recency.
      document.body.append(oldFind, oldHelp);
      const fresh = document.createElement("button");
      document.body.append(fresh);
      const freshDismiss = vi.fn(() => true);
      unregisterFresh = registerTransientLayer({ id: "fresh-after-cleanup", root: () => fresh, dismiss: freshDismiss });
      oldFind.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
      oldFind.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      oldHelp.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
      oldHelp.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      expect(topTransientLayer()?.id).toBe("fresh-after-cleanup");

      const event = escape();
      window.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(true);
      expect(freshDismiss).toHaveBeenCalledOnce();
    } finally {
      unregisterFresh();
      uninstall();
      // `dispose` is intentionally idempotent: the explicit second call also
      // proves the test does not rely on the final assertion reset for cleanup.
      dispose();
      addSpy.mockRestore();
      removeSpy.mockRestore();
    }
  });

  it("keeps a mounted same-id replacement distinct from its stale disposer and old root", async () => {
    const oldRoot = document.createElement("button");
    const replacementRoot = document.createElement("button");
    const sentinelRoot = document.createElement("button");
    const oldTrigger = document.createElement("button");
    const replacementTrigger = document.createElement("button");
    document.body.append(oldRoot, replacementRoot, sentinelRoot, oldTrigger, replacementTrigger);
    const oldTriggerFocus = vi.spyOn(oldTrigger, "focus");
    const replacementTriggerFocus = vi.spyOn(replacementTrigger, "focus");
    const replacementDismiss = vi.fn(() => { unregisterReplacement(); return true; });
    const unregisterOld = registerTransientLayer({
      id: "same-id-owner",
      root: () => oldRoot,
      trigger: () => oldTrigger,
      dismiss: () => true,
    });
    let unregisterReplacement = registerTransientLayer({
      id: "same-id-owner",
      root: () => replacementRoot,
      trigger: () => replacementTrigger,
      dismiss: replacementDismiss,
    });
    const unregisterSentinel = registerTransientLayer({ id: "newer-sentinel", root: () => sentinelRoot, dismiss: () => true });
    try {
      unregisterOld();
      oldRoot.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
      oldRoot.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      expect(topTransientLayer()?.id).toBe("newer-sentinel");
      expect(replacementDismiss).not.toHaveBeenCalled();
      expect(oldTriggerFocus).not.toHaveBeenCalled();

      replacementRoot.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
      expect(topTransientLayer()?.id).toBe("same-id-owner");
      sentinelRoot.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      replacementRoot.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      expect(topTransientLayer()?.id).toBe("same-id-owner");

      expect(dismissTopTransient("escape")).toBe(true);
      await tick();
      expect(replacementDismiss).toHaveBeenCalledOnce();
      expect(replacementTriggerFocus).toHaveBeenCalledOnce();
      expect(document.activeElement).toBe(replacementTrigger);
      expect(oldTriggerFocus).not.toHaveBeenCalled();
    } finally {
      unregisterOld();
      unregisterReplacement();
      unregisterSentinel();
      oldTriggerFocus.mockRestore();
      replacementTriggerFocus.mockRestore();
    }
  });

  it("prunes and consumes one mounted false/stale callback before a later Escape reaches the lower owner", () => {
    const falseRoot = document.createElement("button");
    document.body.append(falseRoot);
    const lowerDismiss = vi.fn(() => true);
    const unregisterLower = registerTransientLayer({ id: "lower-after-stale", dismiss: lowerDismiss });
    const unregisterFalse = registerTransientLayer({ id: "false-top", root: () => falseRoot, dismiss: () => false });
    const uninstall = installKeybindings();
    try {
      const first = escape();
      window.dispatchEvent(first);
      expect(first.defaultPrevented).toBe(true);
      expect(lowerDismiss).not.toHaveBeenCalled();

      const second = escape();
      window.dispatchEvent(second);
      expect(second.defaultPrevented).toBe(true);
      expect(lowerDismiss).toHaveBeenCalledOnce();
    } finally {
      uninstall();
      unregisterFalse();
      unregisterLower();
    }
  });

  it("peels a real Lightbox menu, Lightbox, and lower owner by Escape, then repeats the identical states through Back", async () => {
    const openMenu = async () => {
      setLightbox("data:image/png;base64,AA==");
      await tick();
      const image = document.querySelector<HTMLImageElement>(".lightbox-img")!;
      image.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 12, clientY: 24 }));
      await tick();
      expect(document.querySelector(".lightbox-menu")).not.toBeNull();
    };
    const firstHost = mountedHost();
    const firstDispose = render(() => <Lightbox />, firstHost);
    const firstLower = vi.fn(() => true);
    const unregisterFirstLower = registerTransientLayer({ id: "lightbox-escape-lower", dismiss: firstLower });
    const uninstall = installKeybindings();
    let secondDispose = () => {};
    let unregisterSecondLower = () => {};
    try {
      await openMenu();
      for (const expected of ["menu", "lightbox", "lower"] as const) {
        const event = escape();
        window.dispatchEvent(event);
        await tick();
        expect(event.defaultPrevented).toBe(true);
        if (expected === "menu") {
          expect(document.querySelector(".lightbox-menu")).toBeNull();
          expect(lightbox()).not.toBeNull();
        } else if (expected === "lightbox") {
          expect(lightbox()).toBeNull();
        } else {
          expect(firstLower).toHaveBeenCalledOnce();
        }
      }
      unregisterFirstLower();
      firstDispose();

      const secondHost = mountedHost();
      secondDispose = render(() => <Lightbox />, secondHost);
      const secondLower = vi.fn(() => true);
      unregisterSecondLower = registerTransientLayer({ id: "lightbox-back-lower", dismiss: secondLower });
      await openMenu();
      expect(dismissTopTransient("back")).toBe(true);
      await tick();
      expect(document.querySelector(".lightbox-menu")).toBeNull();
      expect(lightbox()).not.toBeNull();
      expect(dismissTopTransient("back")).toBe(true);
      await tick();
      expect(lightbox()).toBeNull();
      expect(dismissTopTransient("back")).toBe(true);
      expect(secondLower).toHaveBeenCalledOnce();
    } finally {
      unregisterSecondLower();
      secondDispose();
      unregisterFirstLower();
      firstDispose();
      uninstall();
      setLightbox(null);
    }
  });
});
