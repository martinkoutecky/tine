import { createRoot, createSignal } from "solid-js";
import { expect, it, vi } from "vitest";
import { createGraphResource } from "./graphResource";
import { bumpGraphEpoch } from "./graphSession";
import { graphOwner, readOwned } from "./owned";

const flush = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); };

it("asks again when the epoch retires the outstanding read, and shows the new answer", async () => {
  let reply!: (value: string) => void;
  const work = vi.fn()
    .mockImplementationOnce(() => new Promise<string>((resolve) => { reply = resolve; }))
    .mockResolvedValue("now");
  await createRoot(async (dispose) => {
    const [resource] = createGraphResource(() => "key", async () => {
      const read = await readOwned(graphOwner(), work());
      return read.kind === "current" ? read.value : "retired";
    });
    await flush();
    bumpGraphEpoch();
    reply("before");
    await vi.waitFor(() => expect(resource()).toBe("now"));
    expect(work).toHaveBeenCalledTimes(2);
    dispose();
  });
});

it("reads once per value and epoch, and not at all for a skipped source", async () => {
  const work = vi.fn(async (key: string) => key);
  await createRoot(async (dispose) => {
    const [key, setKey] = createSignal<string | null>(null);
    const [tick, setTick] = createSignal(0);
    const [resource] = createGraphResource(() => { tick(); return key(); }, work);
    await flush();
    expect(work).not.toHaveBeenCalled();
    setKey("a"); await flush();
    setTick(1); await flush(); // the source re-ran with an equal value
    expect(work).toHaveBeenCalledTimes(1);
    bumpGraphEpoch(); await flush();
    expect(work).toHaveBeenCalledTimes(2);
    expect(resource()).toBe("a");
    dispose();
  });
});
