import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore, setRaw } from "../document";
import { loadSingle } from "../document/workingSet";
import { bumpDataRev } from "../graphSession";
import { AstBody } from "./body";
import { initParser } from "./parse";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("live inline block references (GH #166)", () => {
  it("refreshes the visible reference text after the source block is edited", async () => {
    const id = "16600000-0000-4000-8000-000000000001";
    const originalText = "Original referenced text";
    const updatedText = "Updated referenced text";
    const target = {
      id,
      raw: `${originalText}\nid:: ${id}`,
      collapsed: false,
      children: [],
      properties: [["id", id]] as [string, string][],
    };
    loadSingle({
      kind: "page",
      name: "Reference source",
      title: "Reference source",
      pre_block: null,
      blocks: [target],
    });
    const resolvedGroup = {
      page: "Reference source",
      kind: "page" as const,
      blocks: [target],
    };
    const resolveBlocks = vi.spyOn(backend(), "resolveBlocks")
      .mockResolvedValueOnce([resolvedGroup])
      .mockResolvedValueOnce([null]);

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AstBody raw={`See ((${id}))`} />, host);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent).toBe(originalText));
      expect(resolveBlocks).toHaveBeenCalledTimes(1);

      setRaw(id, `${updatedText}\nid:: ${id}`);

      await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent).toBe(updatedText));
      expect(resolveBlocks).toHaveBeenCalledTimes(1);

      // A later deletion is authoritative even if the old entity remains in the
      // bounded frontend working set until its page is evicted.
      bumpDataRev();
      await vi.waitFor(() => {
        expect(host.querySelector(".block-ref")?.classList.contains("block-ref-missing")).toBe(true);
        expect(host.querySelector(".block-ref")?.textContent).not.toContain(updatedText);
        expect(resolveBlocks).toHaveBeenCalledTimes(2);
      });
    } finally {
      dispose();
    }
  });

  it("batch-refreshes visible unloaded UUIDs after external edits and deletion", async () => {
    const id = "16600000-0000-4000-8000-000000000002";
    const group = (text: string) => ({
      page: "Unloaded source",
      kind: "page" as const,
      blocks: [{ id, raw: `${text}\nid:: ${id}`, collapsed: false, children: [] }],
    });
    const resolveBlocks = vi.spyOn(backend(), "resolveBlocks")
      .mockResolvedValueOnce([group("Original unloaded text")])
      .mockResolvedValueOnce([group("Externally updated text")])
      .mockResolvedValueOnce([null]);

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AstBody raw={`See ((${id})) twice ((${id}))`} />, host);
    try {
      await vi.waitFor(() => expect(host.querySelectorAll(".block-ref")[0]?.textContent).toBe("Original unloaded text"));
      expect(host.querySelectorAll(".block-ref")).toHaveLength(2);
      expect(resolveBlocks).toHaveBeenCalledTimes(1);
      expect(resolveBlocks).toHaveBeenLastCalledWith([id]);

      // This is the same signal an external watcher transaction or landed save
      // emits. The source never enters doc.byId, so only UUID re-resolution can
      // make the visible references current.
      bumpDataRev();
      await vi.waitFor(() => expect(host.querySelectorAll(".block-ref")[0]?.textContent).toBe("Externally updated text"));
      expect(host.querySelectorAll(".block-ref")[1]?.textContent).toBe("Externally updated text");
      expect(resolveBlocks).toHaveBeenCalledTimes(2);
      expect(resolveBlocks).toHaveBeenLastCalledWith([id]);

      bumpDataRev();
      await vi.waitFor(() => expect(host.querySelectorAll(".block-ref-missing")).toHaveLength(2));
      expect([...host.querySelectorAll(".block-ref")].map((element) => element.textContent))
        .not.toContain("Externally updated text");
      expect(resolveBlocks).toHaveBeenCalledTimes(3);
      expect(resolveBlocks).toHaveBeenLastCalledWith([id]);
    } finally {
      dispose();
    }
  });
});

// GH #589 (master 34272cf6b): OG resolves a block reference only when its id is
// a UUID and shows an unresolved one as its source text. `(((uuid)))` parses
// (mldoc and lsdoc alike) as the id `(uuid` plus a plain `)`.
describe("unresolved inline block references (GH #589)", () => {
  it("shows the full source text, and does not look up an id that is not a UUID", async () => {
    const id = "58900000-0000-4000-8000-000000000001";
    const resolveBlocks = vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AstBody raw={`a ((${id})) b (((${id}))) c`} />, host);
    try {
      await vi.waitFor(() => expect(host.querySelectorAll(".block-ref-missing")).toHaveLength(2));
      await vi.waitFor(() => expect(resolveBlocks).toHaveBeenCalled());
      const refs = [...host.querySelectorAll(".block-ref")].map((element) => element.textContent);
      expect(refs).toEqual([`((${id}))`, `(((${id}))`]);
      expect(host.textContent).toContain(`(((${id})))`);
      expect(resolveBlocks.mock.calls.flatMap(([ids]) => ids)).toEqual([id]);
    } finally {
      dispose();
    }
  });
});
