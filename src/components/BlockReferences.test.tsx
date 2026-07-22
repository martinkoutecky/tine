import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setDoc } from "../store";
import { bumpDataRev } from "../ui";

vi.mock("./LiveRefGroup", () => ({
  LiveRefGroup: () => null,
}));

afterEach(() => {
  vi.restoreAllMocks();
  setDoc({ byId: {}, pages: [], feed: [], loaded: false });
  document.body.innerHTML = "";
});

describe("block referrer panel durable identity (GH #154)", () => {
  it("queries a fresh target's durable UUID and reports two distinct referrer blocks", async () => {
    const durable = "12345678-1234-4234-8234-123456789abc";
    const transient = "bfresh-panel";
    setDoc({
      byId: {
        [transient]: {
          id: transient,
          raw: `Fresh panel target\nid:: ${durable}`,
          collapsed: false,
          parent: null,
          page: "Target page",
          children: [],
        },
      },
      pages: [{
        name: "Target page",
        kind: "page",
        title: "Target page",
        preBlock: null,
        roots: [transient],
        format: "md",
        readOnly: false,
        guide: false,
      }],
      feed: ["Target page"],
      loaded: true,
    });
    const getReferrers = vi.spyOn(backend(), "getBlockReferrers").mockResolvedValue([{
      page: "Referrers",
      kind: "page",
      blocks: [
        { id: "referrer-one", raw: `((${durable}))`, collapsed: false, children: [] },
        { id: "referrer-two", raw: `((${durable}))`, collapsed: false, children: [] },
      ],
    }]);
    const { BlockReferences } = await import("./BlockReferences");
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <BlockReferences id={transient} />, root);

    try {
      bumpDataRev();
      await vi.waitFor(() => {
        expect(getReferrers).toHaveBeenCalledWith(durable);
        expect(root.textContent).toContain("2 Linked References");
      });
    } finally {
      dispose();
    }
  });
});
