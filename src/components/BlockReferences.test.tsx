import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setDoc } from "../store";
import { bumpDataRev, requestBlockReferences, setBlockReferencesRequest } from "../ui";
import { initParser } from "../render/parse";
import { Block } from "./Block";

vi.mock("./LiveRefGroup", () => ({
  LiveRefGroup: () => null,
}));

beforeAll(() => initParser());

afterEach(() => {
  vi.restoreAllMocks();
  setBlockReferencesRequest(null);
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

  it("opens a live runtime block for the authored highlight reference requested by PDF", async () => {
    const runtimeId = "runtime-highlight-block";
    const authoredId = "authored-highlight-id";
    setDoc({
      byId: {
        [runtimeId]: {
          id: runtimeId,
          raw: `Highlight annotation\nid:: ${authoredId}`,
          collapsed: false,
          parent: null,
          page: "hls__paper",
          children: [],
        },
      },
      pages: [{
        name: "hls__paper",
        kind: "page",
        title: "hls__paper",
        preBlock: null,
        roots: [runtimeId],
        format: "md",
        readOnly: false,
        guide: false,
      }],
      feed: ["hls__paper"],
      loaded: true,
    });
    const getReferrers = vi.spyOn(backend(), "getBlockReferrers").mockResolvedValue([{
      page: "Referrers",
      kind: "page",
      blocks: [{ id: "referrer", raw: `((${authoredId}))`, collapsed: false, children: [] }],
    }]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Block id={runtimeId} />, root);

    try {
      expect(root.querySelector(".ls-block")?.getAttribute("data-block-id")).toBe(runtimeId);
      expect(root.querySelector(".ls-block")?.getAttribute("data-block-ref")).toBe(authoredId);
      requestBlockReferences(authoredId);
      await vi.waitFor(() => {
        expect(getReferrers).toHaveBeenCalledWith(authoredId);
        expect(root.textContent).toContain("1 Linked Reference");
      });
    } finally {
      dispose();
    }
  });
});
