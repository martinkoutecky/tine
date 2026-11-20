import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { pageByName, setDoc } from "../document/model";
import { RefBlocks } from "./RefBlocks";
import { LiveRefGroup } from "./LiveRefGroup";
import { setGraphMeta } from "../graphSession";
import { InlineText } from "../render/inline";
import { backend } from "../backend";
import { ExportModal } from "./ExportModal";
import { closeExportModal, openExportModal } from "../ui";
import { StickyAncestor } from "./StickyAncestor";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import { pageIdentityKey } from "../ui";

afterEach(() => {
  closeExportModal(); vi.restoreAllMocks(); localStorage.clear();
  resetPageIndex();
  setGraphMeta(null);
  setDoc({ byId: {}, pages: [], feed: [], loaded: false });
});

it.each(["ref", "query"] as const)("waits for the source format before rendering a cold %s row", async (surface) => {
  const sources = [
    { page: "Org source", file: "Org source.ORG", raw: "/Org emphasis/", label: "Org emphasis" },
    { page: "Markdown source", file: "Markdown source.md", raw: "_Markdown emphasis_", label: "Markdown emphasis" },
  ];
  setGraphMeta({ root: "/synthetic", preferred_format: "md" } as NonNullable<Parameters<typeof setGraphMeta>[0]>);
  vi.spyOn(backend(), "getPage").mockImplementation(() => new Promise(() => {}));
  vi.spyOn(backend(), "pageInventory").mockImplementation(() => new Promise(() => {}));
  const host = document.createElement("div");
  const dispose = render(() => <>{sources.map(({ page, raw }) => <LiveRefGroup page={page} kind="page" surface={surface}
    eager blocks={[{ id: page, raw, children: [], collapsed: false }]} />)}</>, host);
  try {
    expect(host.textContent).not.toContain("/Org emphasis/");
    vi.mocked(backend().pageInventory).mockResolvedValue({ rev: "1", entries: sources.map(({ page, file }) => ({
      name: page, key: pageIdentityKey(page), is_journal: false, day: null,
      target: { kind: "existing", id: `pages/${file}`, others: [] },
    })) });
    await refreshPageIndex();
    expect([...host.querySelectorAll("em")].map(el => el.textContent)).toEqual(sources.map(source => source.label));
    expect(sources.map(({ page }) => pageByName(page))).toEqual([undefined, undefined]);
    expect(backend().getPage).toHaveBeenCalledTimes(sources.length);
  } finally { dispose(); }
});

it("renders a cold result whose physical path already gives its format", () => {
  setGraphMeta({ root: "/synthetic", preferred_format: "md" } as NonNullable<Parameters<typeof setGraphMeta>[0]>);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(() => new Promise(() => {}));
  const host = document.createElement("div");
  const dispose = render(() => <LiveRefGroup page="Org source" kind="page" surface="query" eager path="pages/Org source.ORG"
    blocks={[{ id: "org-note", raw: "/Org emphasis/", children: [], collapsed: false }]} />, host);
  try {
    expect(host.querySelector("em")?.textContent).toBe("Org emphasis");
    expect(pageByName("Org source")).toBeUndefined();
  } finally { dispose(); }
});

it("shows an Org backlink's body", () => {
  setDoc("pages", [{ name: "Org notes", title: "Org notes", kind: "page", preBlock: null,
    roots: [], format: "org", readOnly: false, guide: false }]);
  const host = document.createElement("div");
  const dispose = render(() => <RefBlocks page="Org notes" blocks={[{
    id: "org-note", raw: "Note:: see [[Foo]]", children: [], collapsed: false,
  }]} />, host);
  try { expect(host.textContent).toContain("Note:: see [[Foo]]"); }
  finally { dispose(); }
});

it("keeps Org body text in an ancestor label", () => {
  const raw = "Note:: see [[Foo]]";
  setDoc({ byId: { ancestor: { id: "ancestor", raw, collapsed: false, children: [], parent: null, page: "Org notes" } },
    pages: [{ name: "Org notes", title: "Org notes", kind: "page", preBlock: null,
      roots: ["ancestor"], format: "org", readOnly: false, guide: false }], feed: [], loaded: true });
  const host = document.createElement("div");
  const dispose = render(() => <StickyAncestor id="ancestor" raw={raw} />, host);
  try { expect(host.querySelector(".sticky-ancestor-label")?.textContent).toContain(raw); }
  finally { dispose(); }
});

it.each([false, true])("shows an Org block reference and exports its body (loaded=%s)", async (loaded) => {
  const id = loaded ? "11111111-1111-4111-8111-111111111111" : "22222222-2222-4222-8222-222222222222";
  const raw = "Note:: see [[Foo]]";
  setDoc({
    byId: { host: { id: "host", raw: `((${id}))`, children: [], parent: null, page: "Host", collapsed: false },
      ...(loaded ? { [id]: { id, raw, children: [], parent: null, page: "Org notes", collapsed: false } } : {}) },
    pages: [
      { name: "Host", title: "Host", kind: "page", preBlock: null, roots: ["host"], format: "md", readOnly: false, guide: false },
      ...(loaded ? [{ name: "Org notes", title: "Org notes", kind: "page" as const, preBlock: null, roots: [id], format: "org" as const, readOnly: false, guide: false }] : []),
    ], feed: ["Host"], loaded: true,
  });
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [{
    name: "Org notes", key: pageIdentityKey("Org notes"), is_journal: false, day: null,
    target: { kind: "existing", id: "pages/Org notes.ORG", others: [] },
  }] });
  await refreshPageIndex();
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => ({
    page: "Org notes", kind: "page", blocks: [{ id, raw, children: [], collapsed: false }],
  })));
  const host = document.createElement("div");
  const dispose = render(() => <><InlineText text={`((${id}))`} /><ExportModal /></>, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent).toContain(raw));
    openExportModal(["host"]);
    await vi.waitFor(() => expect(host.querySelector<HTMLTextAreaElement>(".export-preview")?.value).toContain("Note:: see"));
  } finally { dispose(); }
});
