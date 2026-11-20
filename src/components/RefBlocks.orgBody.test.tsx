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
import "../graph";
import { renamePageOnDisk } from "../document";

afterEach(() => {
  closeExportModal(); vi.restoreAllMocks(); localStorage.clear();
  resetPageIndex();
  setGraphMeta(null);
  setDoc({ byId: {}, pages: [], feed: [], loaded: false });
});

it.each(["ref", "query"] as const)("renders both source formats on first display of cold %s rows", async (surface) => {
  const sources = [
    { page: "Org source", file: "Org source.ORG", raw: "/Org emphasis/", label: "Org emphasis" },
    { page: "Markdown source", file: "Markdown source.md", raw: "_Markdown emphasis_", label: "Markdown emphasis" },
  ];
  setGraphMeta({ root: "/synthetic", preferred_format: "md" } as NonNullable<Parameters<typeof setGraphMeta>[0]>);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(() => new Promise(() => {}));
  vi.spyOn(backend(), "pageInventory").mockImplementation(() => new Promise(() => {}));
  const host = document.createElement("div");
  const dispose = render(() => <>{sources.map(({ page, file, raw }) => <LiveRefGroup page={page} kind="page" path={`pages/${file}`} surface={surface}
    eager blocks={[{ id: page, raw, children: [], collapsed: false }]} />)}</>, host);
  try {
    expect([...host.querySelectorAll("em")].map(el => el.textContent)).toEqual(sources.map(source => source.label));
    vi.mocked(backend().pageInventory).mockResolvedValue({ rev: "1", entries: sources.map(({ page, file }) => ({
      name: page, key: pageIdentityKey(page), is_journal: false, day: null,
      target: { kind: "existing", id: `pages/${file}`, others: [] },
    })) });
    await refreshPageIndex();
    expect([...host.querySelectorAll("em")].map(el => el.textContent)).toEqual(sources.map(source => source.label));
    expect(sources.map(({ page }) => pageByName(page))).toEqual([undefined, undefined]);
    expect(backend().getPageByPath).toHaveBeenCalledTimes(sources.length);
  } finally { dispose(); }
});

it.each(["ref", "query"] as const)("keeps %s bodies and breadcrumbs mounted across an unrelated rename", async (surface) => {
  const sources = [
    { page: "Org source", kind: "page" as const, path: "pages/Org source.ORG" },
    { page: "Journal source", kind: "journal" as const, path: "journals/2026_11_20.ORG" },
  ];
  setGraphMeta({ root: "/synthetic", preferred_format: "md" } as NonNullable<Parameters<typeof setGraphMeta>[0]>);
  const byName = vi.spyOn(backend(), "getPage").mockResolvedValue(null);
  const byPath = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  const inventory = vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: sources.map(({ page, kind, path }) => ({
    name: page, key: pageIdentityKey(page), is_journal: kind === "journal", day: null,
    target: { kind: "existing", id: path, others: [] },
  })) });
  await refreshPageIndex();
  vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "renamed",
    renamed: [{ from: "Other", to: "Renamed", path: null }], touched: [] });
  const host = document.createElement("div");
  const dispose = render(() => <>{sources.flatMap(({ page, kind, path }) => [path, undefined].map((owner, i) =>
    <LiveRefGroup page={page} kind={kind} path={owner} surface={surface} eager showBreadcrumb
      blocks={[{ id: `${page}-${i}`, raw: "/Body/", breadcrumb: ["/Parent/"], children: [], collapsed: false }]} />
  ))}</>, host);
  try {
    const bodies = [...host.querySelectorAll(".ref-block")];
    const crumbs = [...host.querySelectorAll(".ref-crumb-label")];
    expect(bodies).toHaveLength(4);
    expect(crumbs).toHaveLength(4);
    const mutations: MutationRecord[] = [];
    const observer = new MutationObserver(records => mutations.push(...records));
    observer.observe(host, { childList: true, subtree: true });
    inventory.mockResolvedValue({ rev: "2", entries: [] });
    const renamed = await renamePageOnDisk("Other", "Renamed");
    expect(renamed).toBe("renamed");
    await refreshPageIndex();
    const mounts = [...mutations, ...observer.takeRecords()].flatMap(record => [...record.addedNodes]);
    observer.disconnect();
    expect(mounts.filter(node => node instanceof Element &&
      (node.matches(".ref-block, .ref-crumb-label") || node.querySelector(".ref-block, .ref-crumb-label")))).toHaveLength(0);
    expect([...host.querySelectorAll(".ref-block")]).toEqual(bodies);
    expect([...host.querySelectorAll(".ref-crumb-label")]).toEqual(crumbs);
    expect(bodies.map(body => body.querySelector(".block-content")?.textContent?.trim())).toEqual(["Body", "/Body/", "Body", "/Body/"]);
    expect(crumbs.map(crumb => crumb.textContent)).toEqual(["Parent", "/Parent/", "Parent", "/Parent/"]);
    expect(byName.mock.calls.length + byPath.mock.calls.length).toBe(4);
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
