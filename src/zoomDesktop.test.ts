// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";

const nativeZoom = vi.fn(() => Promise.resolve());
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ setZoom: nativeZoom }), Webview: class {} }));
vi.mock("./backend", async original => ({ ...(await original<object>()), isTauri: () => true }));
vi.mock("./platform", () => ({ platformKind: () => Promise.resolve("linux") }));

const { applyAndroidInterfaceZoom, applyZoom, zoomIn, zoomReset } = await import("./zoom");

afterEach(async () => {
  zoomReset();
  await vi.waitFor(() => expect(document.documentElement.style.zoom).toBe(""));
  nativeZoom.mockClear();
});

it("GH #826 keeps desktop zoom native and clears mobile viewport compensation", async () => {
  applyAndroidInterfaceZoom(1.5);
  applyZoom();
  await vi.waitFor(() => expect(nativeZoom).toHaveBeenLastCalledWith(1));
  zoomIn();
  await vi.waitFor(() => expect(nativeZoom).toHaveBeenLastCalledWith(1.1));
  expect(document.documentElement.style.zoom).toBe("");
  expect(document.documentElement.style.getPropertyValue("--interface-zoom")).toBe("");
});
