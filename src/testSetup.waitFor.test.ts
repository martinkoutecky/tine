import { describe, expect, it, vi } from "vitest";

/** The worker not scheduled for `ms`: every timer due meanwhile then fires at once, in expiry order. */
function stall(ms: number) {
  const until = performance.now() + ms;
  while (performance.now() < until) { /* not scheduled */ }
}

describe("vi.waitFor across a stalled worker (card HBaE8)", () => {
  it("judges the condition at its deadline, not by the sample taken before the stall", async () => {
    let shown = false;
    // The picker tests' shape: a 90 ms debounce armed just before the wait begins.
    setTimeout(() => { shown = true; }, 90);
    const waiting = vi.waitFor(() => expect(shown).toBe(true));
    stall(1100);
    await waiting;
  });

  it("still fails a condition that is false at its deadline", async () => {
    await expect(vi.waitFor(() => expect("absent").toBe("shown"), { timeout: 100 }))
      .rejects.toThrow(/expected 'absent' to be 'shown'/);
  });

  it("is installed in the node and render configs (the CM6 config extends render)", async () => {
    for (const name of ["vitest.config.ts", "vitest.render.config.ts"]) {
      const { default: config } = await import(/* @vite-ignore */ `../${name}`);
      expect(config.test.setupFiles, name).toContain("./src/testSetup.waitFor.ts");
    }
  });
});
