import { describe, expect, it, vi } from "vitest";

vi.mock("./backend", () => ({ backend: () => ({ graphBindingGeneration: () => 1 }) }));
vi.mock("./toasts", () => ({ pushToast: vi.fn() }));

import { changeGraphSetting } from "./graphPreferences";
import { graphMeta, setGraphMeta } from "./graphSession";
import type { GraphMeta } from "./types";

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("graph preference rollback", () => {
  it("a failed write after an external config edit restores what the file holds", async () => {
    setGraphMeta({ root: "/g", enable_tooltip: true } as unknown as GraphMeta);
    // Turn previews off; the write succeeds.
    changeGraphSetting("enable_tooltip", false, () => Promise.resolve(), "previews");
    await settle();
    expect(graphMeta()?.enable_tooltip).toBe(false);
    // An external editor turns them back on; the config reload publishes it.
    setGraphMeta({ ...graphMeta()!, enable_tooltip: true });
    // The next off-toggle fails: the rollback must restore the file's value (on), not the older off.
    changeGraphSetting("enable_tooltip", false, () => Promise.reject(new Error("disk")), "previews");
    await settle();
    expect(graphMeta()?.enable_tooltip).toBe(true);
  });
});
