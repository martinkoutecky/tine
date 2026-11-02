import { afterEach, describe, expect, it, vi } from "vitest";

const ANDROID_UA =
  "Mozilla/5.0 (Linux; Android 15; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125 Mobile Safari/537.36";
const DESKTOP_UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125 Safari/537.36";

function installBrowserShim(userAgent: string) {
  const listeners: Record<string, ((event: any) => void)[]> = {};
  const history = {
    pushState: vi.fn(),
    back: vi.fn(() => {
      for (const fn of listeners.popstate ?? []) fn({ type: "popstate", state: null });
    }),
  };
  const windowShim = {
    history,
    location: { href: "http://tine.test/" },
    addEventListener: vi.fn((type: string, fn: (event: any) => void) => {
      (listeners[type] ??= []).push(fn);
    }),
    removeEventListener: vi.fn((type: string, fn: (event: any) => void) => {
      listeners[type] = (listeners[type] ?? []).filter((x) => x !== fn);
    }),
  };
  vi.stubGlobal("navigator", {
    userAgent,
    platform: /Android/i.test(userAgent) ? "Linux armv8l" : "Linux x86_64",
  });
  vi.stubGlobal("window", windowShim);
  return { history, windowShim };
}

async function loadRouter() {
  vi.resetModules();
  return await import("./router");
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe("mobile History API bridge", () => {
  it("does not touch browser history on desktop", async () => {
    const { history } = installBrowserShim(DESKTOP_UA);
    const router = await loadRouter();

    router.openPage("Desktop Page");
    router.goBack();

    expect(history.pushState).not.toHaveBeenCalled();
    expect(history.back).not.toHaveBeenCalled();
    expect(router.route()).toEqual({ kind: "journals" });
  });

  it("pushes lightweight history entries for mobile router navigation", async () => {
    const { history, windowShim } = installBrowserShim(ANDROID_UA);
    const router = await loadRouter();

    router.openPage("Page A");
    router.openPage("Page B");

    expect(history.pushState).toHaveBeenCalledTimes(2);
    expect(history.pushState).toHaveBeenLastCalledWith(
      { tineRouter: true },
      "",
      "http://tine.test/"
    );
    expect(windowShim.addEventListener).toHaveBeenCalledTimes(1);
  });

  // L7s8-01: one Back moves one pane.
  it("a native Back undoes the latest navigation, in the pane that made it", async () => {
    const { history } = installBrowserShim(ANDROID_UA);
    const router = await loadRouter();
    const left = router.createPaneRouter("left");
    const right = router.createPaneRouter("right");
    left.openPage("Left page");
    right.openPage("Right page");
    history.back();
    expect(right.route()).toEqual({ kind: "journals" });
    expect(left.route().kind).toBe("page");
    history.back();
    expect(left.route()).toEqual({ kind: "journals" });
  });

  it("an in-app Back moves only the pane it was pressed in", async () => {
    installBrowserShim(ANDROID_UA);
    const router = await loadRouter();
    const left = router.createPaneRouter("left");
    const right = router.createPaneRouter("right");
    left.openPage("Left page");
    right.openPage("Right page");
    left.goBack();
    expect(left.route()).toEqual({ kind: "journals" });
    expect(right.route().kind).toBe("page");
  });

  it("a closed pane no longer answers Back", async () => {
    const { history } = installBrowserShim(ANDROID_UA);
    const router = await loadRouter();
    const left = router.createPaneRouter("left");
    const closed = router.createPaneRouter("closed");
    left.openPage("Left page");
    closed.openPage("Closed page");
    closed.retireMobileHistory();
    history.back();
    expect(closed.route().kind).toBe("page");
    expect(left.route()).toEqual({ kind: "journals" });
  });

  it("does not push when router navigation is a no-op", async () => {
    const { history } = installBrowserShim(ANDROID_UA);
    const router = await loadRouter();

    router.openPage("Page A");
    router.openPage("Page A");

    expect(history.pushState).toHaveBeenCalledTimes(1);
  });

  it("lets popstate drive router back without pushing or double-popping", async () => {
    const { history } = installBrowserShim(ANDROID_UA);
    const router = await loadRouter();
    router.openPage("Page A");
    router.openPage("Page B");

    router.goBack();

    expect(history.back).toHaveBeenCalledTimes(1);
    expect(history.pushState).toHaveBeenCalledTimes(2);
    expect(router.route()).toEqual({ kind: "page", name: "Page A", pageKind: "page" });
  });
});
