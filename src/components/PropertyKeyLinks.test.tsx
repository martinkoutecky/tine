import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PropertyRows } from "../render/PropertyRows";
import { RefBlocks } from "./RefBlocks";
import { QueryPageResults, type QueryPageHit } from "./QueryPageResults";
import { PageView } from "./Page";
import { backend } from "../backend";
import { loadSingle } from "../document/workingSet";
import { resetStore } from "../document";
import { mainPaneRouter } from "../router";
import { setGraphMeta } from "../graphSession";
import type { GraphMeta, PageRead } from "../types";
import * as ui from "../ui";
import * as navigation from "../router";
import { editingId } from "../editorController";
import { Block } from "./Block";
import { QueryPageRows } from "./QueryResultParts";
import { PeekPopup } from "../render/PeekPopup";
import { PropertyKey, visiblePageProperties } from "../render/PropertyKey";

afterEach(() => { vi.restoreAllMocks(); resetStore(); setGraphMeta(null); document.body.innerHTML = ""; });

const hit: QueryPageHit = { entity: "page", page: { name: "Source", kind: "page", date_key: null, path: "pages/Source.md" },
  row: { name: "Source", kind: "page", path: "pages/Source.md", properties: [["owner", "Ada"]] },
  display_text: "Source", evidence: [], score: 0 };

it.each(["block", "live-block", "reference-block", "reference", "query-list", "query-search", "query-dsl", "preview", "page"])("%s keys navigate without entering edit and Shift-click opens the sidebar", async (surface) => {
  const dto: PageRead = { id: "pages/Source.md", name: "Source", title: "Source", kind: "page", format: "md",
    pre_block: "owner:: Ada", blocks: [{ id: "body", raw: "Body\nowner:: Ada", collapsed: false, children: [] }] };
  vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
  if (surface === "page" || surface === "live-block") {
    const { id: _path, ...page } = dto;
    loadSingle(page);
    mainPaneRouter.openPage("Source", "page", { inPlace: true });
  }
  const open = vi.spyOn(navigation, "openPage").mockImplementation(() => {});
  const sidebar = vi.spyOn(ui, "openPageInSidebar").mockImplementation(() => {});
  const edit = vi.fn();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => surface === "block" ? <PropertyRows entries={[["owner", "Ada"]]} format="md" />
    : surface === "live-block" ? <Block id="body" />
    : surface === "reference-block" ? <RefBlocks blocks={dto.blocks} />
    : surface === "reference" ? <RefBlocks blocks={[{ id: "props", raw: "owner:: Ada", page_property: true, collapsed: false, children: [] }]} onEditPageProperties={edit} />
    : surface === "query-dsl" ? <QueryPageRows rows={[hit.row!]} view="list" />
    : surface === "preview" ? <PeekPopup anchor={() => root} blocks={() => []} properties={[["owner", "Ada"]]}
        onPointerEnter={() => {}} onPointerLeave={() => {}} onDismiss={() => {}} />
    : surface === "page" ? <PageView />
    : <QueryPageResults hits={[hit]} showProperties presentation={surface === "query-list" ? "list" : "search"} surfaceId={() => "source"} onOpen={edit} />, root);
  try {
    const key = await vi.waitFor(() => {
      const link = document.querySelector<HTMLAnchorElement>(".prop-key a.page-ref, .query-page-prop-key a.page-ref");
      expect(link?.textContent).toBe("owner"); return link!;
    });
    key.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(open).toHaveBeenCalledWith("owner", "page");
    key.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, shiftKey: true }));
    expect(sidebar).toHaveBeenCalledWith("owner", "page");
    expect(edit).not.toHaveBeenCalled();
    expect(editingId()).toBeNull();
    setGraphMeta({ property_pages_enabled: false } as unknown as GraphMeta);
    expect(document.querySelector(".prop-key a.page-ref, .query-page-prop-key a.page-ref")).toBeNull();
    expect(document.body.textContent).toContain("Ada");
  } finally { dispose(); }
});

it("reacts to disabled property pages without hiding the key or value", () => {
  const root = document.createElement("div");
  const dispose = render(() => <PropertyRows entries={[["owner", "Ada"]]} format="md" />, root);
  try {
    expect(root.querySelector(".prop-key a")).not.toBeNull();
    setGraphMeta({ property_pages_enabled: false } as unknown as GraphMeta);
    expect(root.querySelector(".prop-key a")).toBeNull();
    expect(root.textContent).toBe("owner: Ada");
    setGraphMeta(null);
    expect(root.querySelector(".prop-key a")).not.toBeNull();
  } finally { dispose(); }
});

it("page projections hide built-ins and configured keys, keeping aliases without a separate alias header", () => {
  setGraphMeta({ block_hidden_properties: ["secret"] } as GraphMeta);
  const entries: [string, string][] = [["id", "internal"], ["title", "Source"], ["icon", "X"], ["filters", "{}"],
    ["card-last-score", "5"], ["secret", "hidden"], ["alias", "Other"], ["owner", "Ada"]];
  expect(visiblePageProperties(entries)).toEqual([["alias", "Other"], ["owner", "Ada"]]);
  setGraphMeta({ block_hidden_properties: ["alias"] } as GraphMeta);
  expect(visiblePageProperties(entries).some(([key]) => key === "alias")).toBe(false);
});

it("uses the same normalized key and destination for page and block spellings", () => {
  const root = document.createElement("div"); document.body.append(root);
  const open = vi.spyOn(navigation, "openPage").mockImplementation(() => {});
  const dispose = render(() => <PropertyKey name="Owner_Name" />, root);
  try {
    const key = root.querySelector<HTMLAnchorElement>("a.page-ref")!;
    expect(key.textContent).toBe("owner-name");
    key.click();
    expect(open).toHaveBeenCalledWith("owner-name", "page");
  } finally { dispose(); }
});
