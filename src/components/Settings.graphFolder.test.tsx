import { afterEach, describe, expect, it, vi } from "vitest";
import { Suspense } from "solid-js";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { backend } from "../backend";
import { setGraphMeta } from "../graphSession";
import { setToasts, toasts } from "../toasts";
import type { GraphMeta } from "../types";
import { GraphFolderSetting } from "./GraphFolderSetting";
import * as nativeChrome from "../nativeChrome";

// GH #447 (UI-GRAPH-FOLDER-REVEAL-447): Settings → Graph says where the open
// graph lives and, on desktop, shows that folder in the system file manager.
// Before the fix the Graph section had no folder entry at all; the only route
// was the graph menu's row action.

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  closeSettings();
  vi.restoreAllMocks();
  setGraphMeta(null);
  setToasts([]);
  document.body.innerHTML = "";
  delete (globalThis as { __TINE_PLATFORM__?: string }).__TINE_PLATFORM__;
});

function mountSettings() {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <Suspense fallback={null}><Settings /></Suspense>, root);
  return { root, dispose };
}

const button = (root: HTMLElement, text: string) =>
  [...root.querySelectorAll<HTMLButtonElement>("button")].find((candidate) => candidate.textContent === text);

describe("Settings → Graph shows the graph folder (GH #447)", () => {
  it("names the graph root and reveals it in the file manager", async () => {
    setGraphMeta({ root: "/home/me/notes" } as GraphMeta);
    const reveal = vi.spyOn(backend(), "revealKnownGraph").mockResolvedValue(undefined);
    const { root, dispose } = mountSettings();
    openSettings();
    await tick();
    [...root.querySelectorAll<HTMLButtonElement>(".settings-nav-item")].find((item) => item.textContent === "Graph")!.click();
    await tick();
    const field = root.querySelector('[data-setting-label="Graph folder"]');
    expect(field?.textContent).toContain("/home/me/notes");
    const show = button(root, "Show in folder");
    expect(show, "a Show in folder action in the Graph section").toBeTruthy();
    show!.click();
    await tick();
    expect(reveal).toHaveBeenCalledExactlyOnceWith("/home/me/notes");
    dispose();
  });

  it("a refused reveal is reported, not swallowed", async () => {
    setGraphMeta({ root: "/home/me/notes" } as GraphMeta);
    vi.spyOn(backend(), "revealKnownGraph").mockRejectedValue(new Error("no file manager"));
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <GraphFolderSetting />, root);
    button(root, "Show in folder")!.click();
    await tick();
    await tick();
    expect(toasts().map((toast) => toast.message).join("\n")).toContain("no file manager");
    dispose();
  });

  it("where the build cannot reveal folders, shows the path but offers no action", () => {
    vi.spyOn(nativeChrome, "canRevealInFileManager").mockReturnValue(false);
    setGraphMeta({ root: "/private/var/mobile/notes" } as GraphMeta);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <GraphFolderSetting />, root);
    expect(root.textContent).toContain("/private/var/mobile/notes");
    expect(button(root, "Show in folder")).toBeUndefined();
    dispose();
  });

  // The capability follows the build's platform: the native reveal commands
  // exist for cfg(desktop) only (platform.rs reveal_page_source).
  for (const [platform, expected] of [["desktop", true], ["ios", false], ["android", false]] as const) {
    it(`a ${platform} build ${expected ? "can" : "cannot"} reveal folders`, async () => {
      (globalThis as { __TINE_PLATFORM__?: string }).__TINE_PLATFORM__ = platform;
      vi.resetModules();
      const fresh = await import("../nativeChrome");
      expect(fresh.canRevealInFileManager()).toBe(expected);
    });
  }
});
