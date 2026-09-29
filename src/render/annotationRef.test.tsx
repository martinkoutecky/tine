import type { PageDto, PageRead } from "../types";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { openPdf, layoutPaneIds, paneRouter, resetPaneLayoutToSingle } from "../panes";
import { pdfNavigationIntent, resetPdfNavigationForTest } from "../pdfNavigation";
import type { PdfRoute } from "../router";
import { setDoc } from "../document/model";
import { AnnotationBody } from "../components/AnnotationBody";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { activatePdfOwnership, resetPdfOwnershipForTest } from "../pdfOwnership";

beforeAll(async () => {
  await initParser();
});

beforeEach(() => {
  activatePdfOwnership("/test/annotation-graph");
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  resetPdfNavigationForTest();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  resetPdfOwnershipForTest();
  setDoc("pages", []);
  document.body.replaceChildren();
});

async function settle(): Promise<void> {
  await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
  await Promise.resolve();
}

function currentPdfRoute(): PdfRoute | null {
  for (const id of layoutPaneIds()) {
    const route = paneRouter(id).route();
    if (route.kind === "pdf") return route;
  }
  return null;
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
    } as PageDto as PageRead);

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
      expect(currentPdfRoute()).toMatchObject({ filename: "A_Book.pdf", label: "A_Book.pdf", page: 42 });
      expect(pdfNavigationIntent(currentPdfRoute()!.viewId)()).toMatchObject({ page: 42, highlightId: id });
    } finally {
      dispose();
    }
  });

  it("keeps the current location when a direct link reopens the same PDF", () => {
    const first = openPdf("assets/paper.pdf", "Paper", 7)!;
    paneRouter(layoutPaneIds().find((id) => paneRouter(id).route().kind === "pdf")!).updateActivePdfViewState({ page: 7 });
    const serial = pdfNavigationIntent(first.viewId)()?.serial;
    openPdf("assets/paper.pdf", "Paper");
    expect(currentPdfRoute()).toMatchObject({ filename: "assets/paper.pdf", label: "Paper", page: 7 });
    expect(pdfNavigationIntent(first.viewId)()?.serial).toBe(serial);

    openPdf("assets/paper.pdf", "Paper", 3);
    expect(currentPdfRoute()?.page).toBe(3);
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
      expect(currentPdfRoute()).toMatchObject({ filename: "A_Book.pdf", label: "A_Book.pdf", page: 7 });
      expect(pdfNavigationIntent(currentPdfRoute()!.viewId)()).toMatchObject({ page: 7, highlightId: id });
    } finally {
      dispose();
    }
  });
});
