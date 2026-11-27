import { expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn().mockRejectedValue(new Error("native denied")),
}));

it("rejects a text write when native and browser clipboard transports both fail", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn().mockRejectedValue(new Error("browser denied")) } });
  try {
    const { backend } = await import("./backend");
    await expect(backend().writeText("cut data")).rejects.toThrow("browser denied");
  } finally {
    vi.unstubAllGlobals();
  }
});

it("a text write whose owner retired during the transport's yield dispatches nothing (GH #540)", async () => {
  const native = (await import("@tauri-apps/plugin-clipboard-manager")).writeText as unknown as ReturnType<typeof vi.fn>;
  const browser = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  vi.stubGlobal("navigator", { clipboard: { writeText: browser } });
  try {
    native.mockClear();
    const { backend } = await import("./backend");
    let current = true;
    const write = backend().writeText("- old\r\n", () => current);
    // The owner retires while the transport is still loading.
    current = false;
    await write;
    expect(native).not.toHaveBeenCalled();
    expect(browser).not.toHaveBeenCalled();
    // A current owner: the native transport fails here, so the browser one gets the exact text.
    await backend().writeText("- old\r\n", () => true);
    expect(browser).toHaveBeenCalledWith("- old\r\n");
  } finally {
    vi.unstubAllGlobals();
  }
});
