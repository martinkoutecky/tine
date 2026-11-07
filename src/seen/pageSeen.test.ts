import { expect, it, vi } from "vitest";

it("keeps an empty page context inert before parser initialization", async () => {
  vi.resetModules();
  const { createRoot } = await import("solid-js");
  const { createPageSeen } = await import("./pageSeen");
  const dispose = createRoot(dispose => {
    const seen = createPageSeen(() => null);
    expect(seen.count()).toBe(0);
    expect(seen.position()).toBeNull();
    return dispose;
  });
  dispose();
});
