// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

// UI-IOS-ZOOM-TOAST-ON-LAUNCH: Tauri registers `set_webview_zoom` for desktop
// only, so the native zoom call always rejects on iOS. Every iOS launch showed a
// sticky "Could not apply interface zoom." error toast.
const setZoom = vi.fn(() => Promise.reject(new Error("command set_webview_zoom not found")));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ setZoom }), Webview: class {} }));
vi.mock("./backend", async (orig) => ({ ...(await orig<object>()), isTauri: () => true }));
vi.mock("./platform", () => ({ platformKind: () => Promise.resolve("ios") }));

localStorage.setItem("logseq-claude.zoom", "1.25"); // read once at module load
const { applyZoom } = await import("./zoom");
const { toasts, setToasts } = await import("./toasts");

afterEach(() => {
  setToasts([]);
  setZoom.mockClear();
  document.documentElement.style.zoom = "";
});

describe("iOS interface zoom (UI-IOS-ZOOM-TOAST-ON-LAUNCH)", () => {
  it("applies the stored scale without the desktop-only native zoom and without an error toast", async () => {
    applyZoom();
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
    expect(toasts().map((t) => t.message ?? t)).toEqual([]);
    expect(setZoom).not.toHaveBeenCalled();
    expect(document.documentElement.style.zoom).toBe("1.25");
  });
});
