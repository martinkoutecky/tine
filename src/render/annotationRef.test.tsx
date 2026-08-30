import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setDoc } from "../store";
import { AnnotationBody } from "../components/AnnotationBody";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { layoutPaneIds, openPdf, paneRouter, resetPaneLayoutToSingle } from "../panes";
import { pdfNavigationIntent } from "../pdfNavigation";
import type { PdfRoute } from "../router";

beforeAll(async () => {
  await initParser();
});

beforeEach(() => {
  resetPaneLayoutToSingle({
    tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
    activeIndex: 0,
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  setDoc("pages", []);
  document.body.replaceChildren();
});

function openedPdf(filename?: string): PdfRoute | null {
  for (const paneId of layoutPaneIds()) {
    for (const tab of paneRouter(paneId).tabs()) {
      const route = tab.history[tab.pos];
      if (route.kind === "pdf" && (!filename || route.filename === filename)) return route;
    }
  }
  return null;
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
  await Promise.resolve();
}

describe("PDF annotation block references (GH #61)", () => {
  it("opens and closes through ordinary pane routing without a second PDF authority", async () => {
    const route = openPdf("assets/paper.pdf", "Paper")!;
    expect(openedPdf()).toEqual(route);
    expect(layoutPaneIds()).toHaveLength(2);

    const paneId = layoutPaneIds().find((id) => paneRouter(id).route().kind === "pdf")!;
    await paneRouter(paneId).closePdf();
    expect(openedPdf()).toBeNull();
    expect(layoutPaneIds()).toEqual(["main"]);
  });

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
      const route = openedPdf("A_Book.pdf")!;
      expect(route).toMatchObject({ filename: "A_Book.pdf", label: "A_Book.pdf", page: 42 });
      expect(pdfNavigationIntent(route.viewId)()).toMatchObject({ page: 42, highlightId: id });
    } finally {
      dispose();
    }
  });

  it("keeps the current location when a direct link reopens the same PDF", () => {
    const original = openPdf("assets/paper.pdf", "Paper", 7)!;
    openPdf("assets/paper.pdf", "Paper");
    expect(openedPdf("assets/paper.pdf")).toMatchObject({ viewId: original.viewId, page: 7 });

    openPdf("assets/paper.pdf", "Paper", 3);
    expect(openedPdf("assets/paper.pdf")?.page).toBe(3);
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
      const route = openedPdf("A_Book.pdf")!;
      expect(route).toMatchObject({ filename: "A_Book.pdf", label: "A_Book.pdf", page: 7 });
      expect(pdfNavigationIntent(route.viewId)()).toMatchObject({ page: 7, highlightId: id });
    } finally {
      dispose();
    }
  });
});
