import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { ConflictObject } from "../types";

// GH #490 (master c5279d186): a rejected conflict-comparison read must not throw
// into render (it blanked the whole conflict panel). ConflictResolution reads its
// resource through readOr; the panel keeps its own "Couldn't read" row. `readDiff`
// catches its own errors, so the only way the resource rejects is a throw outside
// its try — the owner lookup here.
vi.mock("../owned", async (original) => ({
  ...(await original<typeof import("../owned")>()),
  graphOwner: () => { throw new Error("owner lookup failed"); },
}));
import { PageConflictResolution } from "./ConflictResolution";

const conflict: ConflictObject = {
  id: "markers:pages/Merged.md",
  source: "vcs-markers",
  page_name: "Merged",
  page_path: "pages/Merged.md",
  kind: "page",
  sides: [{ role: "mine", label: "HEAD" }, { role: "theirs", label: "feature" }],
  block_conflicts: 1,
  markers: ["<<<<<<<", "=======", ">>>>>>>"],
};

afterEach(() => { document.body.innerHTML = ""; });

describe("conflict panel read failure", () => {
  it("shows its own read-failure row instead of throwing into render", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <PageConflictResolution conflict={conflict} />, host);
    await vi.waitFor(() => expect(host.textContent).toContain("Couldn’t read this conflict."));
    expect(host.querySelector(".page-conflict-refusal")).not.toBeNull();
    dispose();
  });
});
