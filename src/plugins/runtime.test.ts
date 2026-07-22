import { describe, expect, it } from "vitest";
import { PluginRuntime } from "./runtime";

class SilentWorker {
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  terminated = false;
  postMessage() {}
  terminate() {
    this.terminated = true;
  }
}

describe("PluginRuntime", () => {
  it("terminates an unresponsive guest at the hard deadline", async () => {
    const worker = new SilentWorker();
    await expect(
      PluginRuntime.create(new ArrayBuffer(0), {
        workerFactory: () => worker,
        initializationTimeoutMs: 5,
      })
    ).rejects.toThrow(/time limit/);
    expect(worker.terminated).toBe(true);
  });
});
