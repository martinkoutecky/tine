import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { setGraphMeta } from "../graphSession";
import type { GraphMeta, PageDto, PageRead } from "../types";
import { loadSingle } from "../document/workingSet";
import { resetStore, blockProperty } from "../document";
import { clearSeededFacets } from "../render/facets";
import { closePageProps, openBlockProps } from "../ui";
import { PageProps } from "./PageProps";
import { PropertyRows } from "../render/PropertyRows";
import { PageView } from "./Page";
import { backend } from "../backend";
import { mainPaneRouter } from "../router";

beforeAll(() => initParser());
afterEach(() => { closePageProps(); resetStore(); setGraphMeta(null); document.body.innerHTML = ""; });

it("renders ordinary editable keys while hiding internal, SRS and configured metadata without an order list", () => {
  setGraphMeta({ block_hidden_properties: ["secret"] } as GraphMeta);
  const host = document.createElement("div");
  const dispose = render(() => <PropertyRows format="md" entries={[
    ["heading", "true"], ["card-last-score", "5"], ["secret", "opaque"],
    ["title", "Title"], ["filters", "{}"], ["icon", "X"], ["owner", "Ada"],
    ["logseq.color", "red"], ["logseq.table.compact", "true"],
  ]} />, host);
  try {
    const keys = [...host.querySelectorAll(".prop-key")].map(el => el.textContent);
    expect(keys).toEqual(["title", "filters", "icon", "owner"]);
  } finally { dispose(); }
});

it("keeps newly hidden keys editable and removable in the block Properties form", () => {
  const keys = ["custom-id", "heading", "background-color", "created-at", "updated-at", "last-modified-at",
    "query-table", "query-properties", "query-sort-by", "query-sort-desc", "ls-type", "hl-type", "hl-page",
    "hl-stamp", "hl-color", "logseq.macro-name", "logseq.macro-arguments", "logseq.tldraw.page", "logseq.tldraw.shape",
    "todo", "doing", "now", "later", "done", "card-last-interval", "card-repeats", "card-last-reviewed",
    "card-next-schedule", "card-ease-factor", "card-last-score", "secret"];
  setGraphMeta({ block_hidden_properties: ["secret"] } as GraphMeta);
  loadSingle({ name: "Properties", title: "Properties", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "b", raw: `Body\n${keys.map(key => `${key}:: original`).join("\n")}`, collapsed: false, children: [] }] });
  clearSeededFacets();
  openBlockProps("b", 20, 20);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <PageProps />, host);
  try {
    expect([...host.querySelectorAll(".pp-field .pp-label")].map(el => el.textContent)).toEqual(expect.arrayContaining(keys));
    for (const key of keys) {
      const field = [...host.querySelectorAll<HTMLElement>(".pp-field")].find(el => el.querySelector(".pp-label")?.textContent === key)!;
      const input = field.querySelector<HTMLInputElement>(".pp-input")!;
      input.focus();
      input.value = "changed";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      input.blur();
      expect(blockProperty("b", key), key).toBe("changed");
      field.querySelector<HTMLButtonElement>(".pp-remove")!.click();
      expect(blockProperty("b", key)).toBeNull();
    }
  } finally { dispose(); }
});

it("filters a real page's pre-block using its own surface and graph config", async () => {
  setGraphMeta({ block_hidden_properties: ["secret"] } as GraphMeta);
  const dto: PageDto = { name: "Properties", title: "Properties", kind: "page", format: "md",
    pre_block: "title:: Properties\nicon:: X\nfilters:: {}\nheading:: true\ncard-last-score:: 5\nsecret:: opaque\nowner:: Ada\nlogseq.color:: red\nlogseq.table.compact:: true",
    blocks: [{ id: "b", raw: "Body", collapsed: false, children: [] }] };
  loadSingle(dto);
  const host = document.createElement("div");
  document.body.append(host);
  vi.spyOn(backend(), "getPage").mockResolvedValue(dto as PageRead);
  mainPaneRouter.openPage("Properties", "page", { inPlace: true });
  const dispose = render(() => <PageView />, host);
  try {
    await new Promise(resolve => setTimeout(resolve, 0));
    await new Promise(resolve => setTimeout(resolve, 0));
    const text = host.querySelector(".page-properties")?.textContent ?? "";
    for (const key of ["heading", "card-last-score", "secret", "title", "icon", "filters"]) expect(text).not.toContain(key);
    for (const key of ["owner", "logseq.color", "logseq.table.compact"]) expect(text).toContain(key);
  } finally { dispose(); vi.restoreAllMocks(); }
});
