import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";
import * as panes from "../panes";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { applySidebarSession } from "../ui";
import { PageView } from "../components/Page";
import { RightSidebar } from "../components/RightSidebar";
import { AstBody } from "./body";

beforeAll(initParser);
afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
  resetStore();
  applySidebarSession({ right: false, items: [] });
});

describe("GH #577: file PDF links are reads through the OS opener (I-2, I-4)", () => {
  for (const [source, destination, format] of [
    ["[paper](file:///tmp/external.pdf)", "file:///tmp/external.pdf", "md"],
    ["[paper](file:///graph/assets/paper.pdf)", "file:///graph/assets/paper.pdf", "md"],
    ["[paper](file:///tmp/a%20paper.PDF)", "file:///tmp/a%20paper.PDF", "md"],
    ["[Test](<file://x:\\Bibliotheek\\Artikelen\\NRC\\iets.pdf>)", "file://x:\\Bibliotheek\\Artikelen\\NRC\\iets.pdf", "md"],
    ["[paper](<file:///C:\\papers\\paper.pdf>)", "file:///C:\\papers\\paper.pdf", "md"],
    ["[paper](file://server/share/paper.pdf)", "file://server/share/paper.pdf", "md"],
    ["[[file:///tmp/external.pdf][paper]]", "file:///tmp/external.pdf", "org"],
    ["![paper](file:///tmp/external.pdf)", "file:///tmp/external.pdf", "md"],
  ] as const) {
    it(`opens ${source} without entering the graph PDF write path`, () => {
      const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
      const reader = vi.spyOn(panes, "openPdf");
      const host = document.createElement("div");
      document.body.append(host);
      const dispose = render(() => <InlineText text={source} format={format} />, host);
      try {
        const link = host.querySelector("a")!;
        expect(link).not.toBeNull();
        link.click();
        expect(external).toHaveBeenCalledExactlyOnceWith(destination);
        expect(asset).not.toHaveBeenCalled();
        expect(reader, "I-4: file links must not enter the annotation writer via panes.openPdf").not.toHaveBeenCalled();
      } finally { dispose(); }
    });
  }

  for (const destination of ["assets/paper.pdf", "./assets/paper.pdf", "../assets/paper.pdf"]) {
    it(`keeps ${destination} in the graph reader`, () => {
      const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      const reader = vi.spyOn(panes, "openPdf").mockReturnValue(null);
      const host = document.createElement("div");
      document.body.append(host);
      const dispose = render(() => <InlineText text={`[paper](${destination})`} />, host);
      try {
        (host.querySelector("a") as HTMLAnchorElement).click();
        expect(reader).toHaveBeenCalledExactlyOnceWith("paper.pdf", "paper");
        expect(external).not.toHaveBeenCalled();
      } finally { dispose(); }
    });
  }

  for (const surface of ["page", "sidebar", "embed"] as const) {
    it(`opens a PDF file link from ${surface} without a reader route`, async () => {
      const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      const reader = vi.spyOn(panes, "openPdf").mockReturnValue(null);
      const page = {
        id: "pages/External PDFs.md", name: "External PDFs", title: "External PDFs", kind: "page" as const,
        pre_block: null,
        blocks: [{ id: "pdf-link-block", raw: "[paper](file:///tmp/external.pdf)", collapsed: false, children: [] }],
      };
      loadSingle(page);
      vi.spyOn(backend(), "getPage").mockResolvedValue(page);
      vi.spyOn(backend(), "getPageByPath").mockResolvedValue(page);
      vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
      vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
      vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
      panes.resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: page.name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
      applySidebarSession({ right: true, items: [{ kind: "page", name: page.name, pageKind: "page" }] });
      const host = document.createElement("div");
      document.body.append(host);
      const dispose = render(() => surface === "page" ? <PageView /> : surface === "sidebar" ? <RightSidebar /> : <AstBody raw="{{embed [[External PDFs]]}}" />, host);
      try {
        const link = await vi.waitFor(() => {
          const link = host.querySelector<HTMLAnchorElement>('a[href="file:///tmp/external.pdf"]');
          expect(link).not.toBeNull();
          return link!;
        });
        link.click();
        expect(external).toHaveBeenCalledExactlyOnceWith("file:///tmp/external.pdf");
        expect(reader).not.toHaveBeenCalled();
      } finally { dispose(); }
    });
  }
});
