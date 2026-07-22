import { describe, expect, it, vi } from "vitest";
import { createCaptureBlurGate, resettleIfVisible } from "./captureVisibility";

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

  it("waits for an asynchronous visible-window readiness barrier", async () => {
    let release!: () => void;
    const barrier = new Promise<void>((resolve) => { release = resolve; });
    const resettle = vi.fn(async () => { await barrier; });
    let finished = false;
    const recovery = resettleIfVisible({ isVisible: async () => true }, resettle).then(() => { finished = true; });

    await Promise.resolve();
    expect(resettle).toHaveBeenCalledOnce();
    expect(finished).toBe(false);
    release();
    await recovery;
    expect(finished).toBe(true);
  });
});

describe("createCaptureBlurGate", () => {
  it("ignores the unfocused transition emitted while a first-show activation is pending", () => {
    const gate = createCaptureBlurGate();

    expect(gate.focusChanged(false)).toBe(false);
    expect(gate.focusChanged(false)).toBe(false);
  });

  it("dismisses once after the capture window has genuinely held focus", () => {
    let now = 1_000;
    const gate = createCaptureBlurGate(() => now, 200);

    expect(gate.focusChanged(true)).toBe(false);
    now += 200;
    expect(gate.focusChanged(false)).toBe(true);
    expect(gate.focusChanged(false)).toBe(false);
  });

  it("ignores transient focus lost while the forwarding process exits, then arms on a stable retry", () => {
    let now = 1_000;
    const gate = createCaptureBlurGate(() => now, 200);

    gate.focusChanged(true);
    now += 40;
    expect(gate.focusChanged(false)).toBe(false);

    now += 80;
    gate.focusChanged(true);
    now += 240;
    expect(gate.focusChanged(false)).toBe(true);
  });

  it("disarms a stale blur after an explicit hide", () => {
    const gate = createCaptureBlurGate();

    gate.focusChanged(true);
    gate.disarm();
    expect(gate.focusChanged(false)).toBe(false);
  });
});
