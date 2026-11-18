import { beforeEach, describe, expect, it, vi } from "vitest";

const reportUiFailure = vi.hoisted(() => vi.fn());
const identity = vi.hoisted(() => ({ value: "0:1" }));
vi.mock("../uiFailure", () => ({ reportUiFailure }));
vi.mock("../binding", () => ({ bindingIdentity: () => identity.value }));
vi.mock("../debug", () => ({ dbg: vi.fn() }));
vi.mock("../backend", () => ({ backend: () => ({}) }));
vi.mock("../document", () => ({ node: () => null, seenPageSnapshot: () => null }));
vi.mock("../inputCapability", () => ({ touchPrimary: () => false }));
vi.mock("../owned", () => ({ graphOwner: () => () => true, readOwned: () => Promise.resolve({ kind: "stale" }) }));

import { reportBlockHistoryFailure } from "./bulletHistory";

describe("block history failures (GH #980)", () => {
  beforeEach(() => { reportUiFailure.mockClear(); });

  it("shows a repeated failure once per graph binding", () => {
    identity.value = "0:1";
    reportBlockHistoryFailure("a");
    reportBlockHistoryFailure("b");
    reportBlockHistoryFailure("c");
    expect(reportUiFailure).toHaveBeenCalledTimes(1);
    expect(reportUiFailure).toHaveBeenCalledWith("block-history", "a");
    identity.value = "0:2";
    reportBlockHistoryFailure("d");
    expect(reportUiFailure).toHaveBeenCalledTimes(2);
  });
});
