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
