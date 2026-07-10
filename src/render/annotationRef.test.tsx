import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { pdfTarget, setPdfTarget } from "../ui";
import { AstBody } from "./body";
import { initParser } from "./parse";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  setPdfTarget(null);
  document.body.replaceChildren();
});

async function settle(): Promise<void> {
  await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
  await Promise.resolve();
}

describe("PDF annotation block references (GH #61)", () => {
  it("opens the owning PDF at hl-page on a plain click", async () => {
    const id = "61a00000-0000-0000-0000-000000000001";
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{
      page: "hls__book",
      kind: "page",
      blocks: [{
        id,
        raw: `Important passage\nhl-page:: 42\nhl-color:: yellow\nls-type:: annotation\nid:: ${id}`,
        collapsed: false,
        children: [],
        properties: [["hl-page", "42"], ["hl-color", "yellow"], ["ls-type", "annotation"], ["id", id]],
      }],
    }]);
    vi.spyOn(backend(), "getPage").mockResolvedValue({
      name: "hls__book",
      kind: "page",
      title: "A Book",
      pre_block: "file:: [A Book](../assets/A_Book.pdf)\nfile-path:: ../assets/A_Book.pdf",
      blocks: [],
    });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AstBody raw={`See ((${id}))`} />, host);
    try {
      await settle();
      const ref = host.querySelector(".block-ref");
      expect(ref).toBeTruthy();
      ref!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
      await settle();

      expect(backend().getPage).toHaveBeenCalledWith("hls__book", "page");
      expect(pdfTarget()).toEqual({ filename: "A_Book.pdf", label: "A_Book.pdf", page: 42 });
    } finally {
      dispose();
    }
  });
});
