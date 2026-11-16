import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { AstBody } from "./body";
import { EmbedMacro } from "../components/EmbedMacro";
import { RefBlocks } from "../components/RefBlocks";
import { Block } from "../components/Block";
import { activatePdfOwnership, resetPdfOwnershipForTest } from "../pdfOwnership";
import { layoutPaneIds, paneRouter, resetPaneLayoutToSingle } from "../panes";
import { pdfNavigationIntent } from "../pdfNavigation";
import { setDoc } from "../document/model";
import { ensurePageLoaded, resetStore } from "../document";
import { setGraphEpoch } from "../graphSession";
import type { BlockDto, Format, PageRead } from "../types";

vi.mock("../assetCache", () => ({
  acquireCachedAssetBlob: () => undefined,
  acquireAssetBlob: vi.fn(async () => ({ url: "blob:crop", release: vi.fn() })),
  assetVersion: () => 0,
}));
import { acquireAssetBlob } from "../assetCache";

const id = "86900000-0000-4000-8000-000000000001";
const page = "hls__nested_book";
let disposers: (() => void)[] = [];
beforeEach(() => {
  setGraphEpoch(value => value + 1);
  activatePdfOwnership("/test/area-869");
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
});
afterEach(() => {
  disposers.forEach(dispose => dispose());
  disposers = [];
  resetStore();
  resetPdfOwnershipForTest();
  vi.restoreAllMocks();
  vi.clearAllMocks();
  document.body.replaceChildren();
});
async function mount(view: () => any) {
  const host = document.createElement("div");
  document.body.append(host);
  disposers.push(render(view, host));
  return host;
}
function fixture(format: Format) {
  const properties: [string, string][] = [["id", id], ["ls-type", "annotation"], ["hl-page", "2"], ["hl-type", "area"], ["hl-stamp", "1730000000000"]];
  const raw = format === "md" ? "\n" + properties.map(([k, v]) => `${k}:: ${v}`).join("\n")
    : `\n:PROPERTIES:\n${properties.map(([k, v]) => `:${k}: ${v}`).join("\n")}\n:END:`;
  const block: BlockDto = { id, raw, properties, collapsed: false, children: [] };
  const source = { name: page, kind: "page", format, title: page, pre_block: format === "md" ? "file-path:: ../assets/nested/book.pdf" : "#+FILE-PATH: ../assets/nested/book.pdf", blocks: [block] } as PageRead;
  vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{ page, kind: "page", blocks: [block] }]);
  vi.spyOn(backend(), "getPage").mockResolvedValue(source);
  return { block, source };
}
describe("PDF area images wherever their block renders (GH #869)", () => {
  for (const format of ["md", "org"] as const) {
    for (const surface of ["reference", "embed", "preview", "source"] as const) {
      it(`${format}: ${surface} shows the crop and opens its exact PDF highlight`, async () => {
        const { block, source } = fixture(format);
        // DTO previews need the owning format even before the source is loaded.
        setDoc("pages", [{ name: page, kind: "page", format, preBlock: source.pre_block, roots: [], readOnly: false, guide: false } as any]);
        if (surface === "source") { setDoc("pages", []); ensurePageLoaded(source); }
        const host = await mount(() => surface === "reference" ? <AstBody raw={`((${id}))`} format={format} />
          : surface === "embed" ? <EmbedMacro body={`embed ((${id}))`} />
          : surface === "preview" ? <RefBlocks blocks={[block]} page={page} /> : <Block id={id} />);
        await vi.waitFor(() => expect(host.querySelector("img")?.getAttribute("src")).toBe("blob:crop"));
        expect(host.textContent).not.toMatch(/:PROPERTIES:|:END:|hl-stamp|hl-type/);
        expect(acquireAssetBlob).toHaveBeenCalledWith(`nested_book/2_${id}_1730000000000.png`);
        host.querySelector("img")!.click();
        await vi.waitFor(() => {
          const route = layoutPaneIds().map(key => paneRouter(key).route()).find(route => route.kind === "pdf");
          expect(route).toMatchObject({ filename: "nested/book.pdf", page: 2 });
          if (route?.kind === "pdf") expect(pdfNavigationIntent(route.viewId)()).toMatchObject({ page: 2, highlightId: id });
        });
      });
    }
  }
  it("keeps an explicit reference alias as its authored label", async () => {
    fixture("md");
    const host = await mount(() => <AstBody raw={`[figure]((${id}))`} />);
    await vi.waitFor(() => expect(host.textContent).toContain("figure"));
    expect(host.querySelector("img")).toBeNull();
  });
});
