import { describe, expect, it, vi } from "vitest";
import { resettleIfVisible } from "./captureVisibility";

describe("resettleIfVisible", () => {
  it("recovers focus setup when the initial capture-shown event was missed", async () => {
    const resettle = vi.fn();

    await resettleIfVisible({ isVisible: async () => true }, resettle);

    expect(resettle).toHaveBeenCalledOnce();
  });

  it("does not focus a capture window that is still hidden", async () => {
    const resettle = vi.fn();

    await resettleIfVisible({ isVisible: async () => false }, resettle);

    expect(resettle).not.toHaveBeenCalled();
  });
});
