import { afterEach, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));
afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); invoke.mockReset(); });

it.each(["android", "desktop"])("uses the supported bounded image transport on %s", async (platform) => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  invoke.mockImplementation(async (cmd: string) => cmd === "app_platform" ? platform : null);
  const { backend } = await import("./backend");
  const bytes = new Uint8Array([137, 80, 78, 71]);
  await backend().copyImageToClipboard(bytes);
  const payload = invoke.mock.calls.find(([cmd]) => cmd === "copy_image_to_clipboard")![1];
  if (platform === "android") {
    expect(payload).toEqual({ bytesB64: "iVBORw==" });
    // Tauri 2.12.3 ipc-protocol.js wraps the payload in a JSON envelope
    // on Android. The string must survive that serialization unchanged.
    expect(JSON.parse(JSON.stringify({ payload })).payload.bytesB64).toBe("iVBORw==");
  } else {
    expect(payload).toBe(bytes);
  }
  invoke.mockClear();
  await expect(backend().copyImageToClipboard(new Uint8Array(64 * 1024 * 1024 + 1))).rejects.toThrow("64 MiB");
  expect(invoke).not.toHaveBeenCalled();
});
