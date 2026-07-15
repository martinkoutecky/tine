import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { openPdf, pdfTarget, setPdfTarget } from "../ui";
import { setDoc } from "../store";
import { AnnotationBody } from "../components/AnnotationBody";
import { AstBody } from "./body";
import { initParser } from "./parse";

beforeAll(async () => {
  await initParser();
});

  afterEach(() => {
  vi.restoreAllMocks();
  setPdfTarget(null);
  setDoc("pages", []);
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
      expect(pdfTarget()).toEqual({
        filename: "A_Book.pdf",
        label: "A_Book.pdf",
        page: 42,
        highlightId: id,
      });
    } finally {
      dispose();
    }
  });

  it("keeps the current location when a direct link reopens the same PDF", () => {
    setPdfTarget({ filename: "assets/paper.pdf", label: "Paper", page: 7 });
    openPdf("assets/paper.pdf", "Paper");
    expect(pdfTarget()).toEqual({
      filename: "assets/paper.pdf",
      label: "Paper",
      page: 7,
    });

    openPdf("assets/paper.pdf", "Paper", 3);
    expect(pdfTarget()?.page).toBe(3);
  });

  it("carries the exact id from a rendered annotation block", async () => {
    const id = "61a00000-0000-0000-0000-000000000002";
    setDoc("pages", [{
      name: "hls__book",
      preBlock: "file-path:: ../assets/A_Book.pdf",
      roots: [],
      format: "markdown",
    } as any]);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => (
      <AnnotationBody
        highlightId={id}
        color="green"
        hlPage={7}
        line="Exact annotation"
        page="hls__book"
      />
    ), host);
    try {
      host.querySelector<HTMLElement>(".hl-prefix")!.click();
      expect(pdfTarget()).toEqual({
        filename: "A_Book.pdf",
        label: "A_Book.pdf",
        page: 7,
        highlightId: id,
      });
    } finally {
      dispose();
    }
  });
});
