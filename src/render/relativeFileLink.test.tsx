import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";
import * as panes from "../panes";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { PageView } from "../components/Page";
import { RightSidebar } from "../components/RightSidebar";
import { AstBody } from "./body";
import { applySidebarSession } from "../ui";
import { graphMeta, setGraphMeta } from "../graphSession";
import { setToasts, toasts } from "../toasts";
import { fileLinkTarget, graphRelative, resolvedFilePath, filePathUrl, fileUrlTitle } from "./relativeFileLink";

beforeAll(initParser);
afterEach(() => {
  vi.restoreAllMocks(); document.body.replaceChildren(); resetStore();
  setToasts([]); setGraphMeta(null); applySidebarSession({ right: false, items: [] });
});

function clickLink(raw: string, format: "md" | "org" = "md") {
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <InlineText text={raw} format={format} />, host);
  const link = host.querySelector<HTMLAnchorElement>("a")!;
  expect(link).not.toBeNull(); link.click(); dispose(); return link;
}

describe("file links resolve from the graph folder (GH #457)", () => {
  for (const [raw, opened, format] of [
    ["[report](files/report.odt)", "files/report.odt", "md"],
    ["[here](notes.txt)", "notes.txt", "md"],
    ["[sub](./sub/notes.txt)", "sub/notes.txt", "md"],
    ["[back](files/old/../a.txt)", "files/a.txt", "md"],
    ["[[file:files/a.txt][org]]", "files/a.txt", "org"],
    ["[paper](docs/paper.pdf)", "docs/paper.pdf", "md"],
  ] as const) {
    it(`opens ${raw} inside the graph`, () => {
      const open = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
      const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      clickLink(raw, format);
      expect(open).toHaveBeenCalledExactlyOnceWith(opened, backend().graphBindingGeneration());
      expect(external).not.toHaveBeenCalled();
    });
  }

  for (const [raw, path, format] of [
    ["[report](../files/report.odt)", "/home/user/files/report.odt", "md"],
    ["[deep](../../files/a.txt)", "/home/files/a.txt", "md"],
    ["[deep](../../../files/a.txt)", "/files/a.txt", "md"],
    ["[spaced](<../files/my file.txt>)", "/home/user/files/my file.txt", "md"],
    ["[escaped](../files/a%20b.txt)", "/home/user/files/a b.txt", "md"],
    // GH #457: Logseq (mldoc) accepts plain spaces in a link destination.
    ["[plain](../files/my report.pdf)", "/home/user/files/my report.pdf", "md"],
    ["[windows](..\\files\\a.txt)", "/home/user/files/a.txt", "md"],
    ["[scheme](file:../files/a.txt)", "/home/user/files/a.txt", "md"],
    ["[[file:../files/a.txt][org]]", "/home/user/files/a.txt", "org"],
    ["[abs](/home/me/report.odt)", "/home/me/report.odt", "md"],
    ["[drive](D:\\Users\\me\\report.odt)", "D:/Users/me/report.odt", "md"],
    ["[abs](/home/me/paper.pdf)", "/home/me/paper.pdf", "md"],
    ["![paper](../files/paper.pdf)", "/home/user/files/paper.pdf", "md"],
    ["[script](../scripts/run.sh)", "/home/user/scripts/run.sh", "md"],
  ] as const) {
    it(`opens ${raw} outside the graph with the file: posture`, () => {
      setGraphMeta({ ...graphMeta()!, root: "/home/user/graph" });
      const open = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
      const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      const reader = vi.spyOn(panes, "openPdf").mockReturnValue(null);
      const link = clickLink(raw, format);
      expect(link.hasAttribute("data-broken-file-link")).toBe(false);
      expect(link.title).toBe(path);
      expect(external).toHaveBeenCalledExactlyOnceWith(filePathUrl(path));
      expect(open).not.toHaveBeenCalled(); expect(reader).not.toHaveBeenCalled();
    });
  }

  it("reports a refused outside open", async () => {
    setGraphMeta({ ...graphMeta()!, root: "/graph" });
    vi.spyOn(backend(), "openExternal").mockRejectedValue("opening local files is available on desktop only");
    clickLink("[file](../gone.txt)");
    await vi.waitFor(() => expect(toasts()[0]?.message).toContain("desktop only"));
  });

  it("keeps the assets route", () => {
    const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    clickLink("[asset](../assets/a.txt)");
    expect(asset).toHaveBeenCalledExactlyOnceWith("a.txt", backend().graphBindingGeneration());
    expect(external).not.toHaveBeenCalled();
  });

  for (const id of ["pages/Plan.md", "journals/2026_10_14.md", "pages/projects/Plan.md"]) {
    it(`uses the graph root rather than ${id}'s directory`, () => {
      setGraphMeta({ ...graphMeta()!, root: "/home/user/graph" });
      loadSingle({ id, name: "Plan", title: "Plan", kind: id.startsWith("journals/") ? "journal" : "page", pre_block: null,
        blocks: [{ id: "rel-link-block", raw: "[file](../files/a.txt)", collapsed: false, children: [] }] });
      const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      clickLink("[file](../files/a.txt)");
      expect(external).toHaveBeenCalledExactlyOnceWith("file:///home/user/files/a.txt");
    });
  }

  for (const surface of ["page", "sidebar", "embed"] as const) {
    it(`opens outside links from ${surface}`, async () => {
      setGraphMeta({ ...graphMeta()!, root: "/home/user/graph" });
      const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      const page = { id: "pages/Plan.md", name: "Plan", title: "Plan", kind: "page" as const, pre_block: null,
        blocks: [{ id: "rel-link-block", raw: "[report](../files/report.odt)", collapsed: false, children: [] }] };
      loadSingle(page);
      vi.spyOn(backend(), "getPage").mockResolvedValue(page);
      vi.spyOn(backend(), "getPageByPath").mockResolvedValue(page);
      vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
      vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
      vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
      panes.resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Plan", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
      applySidebarSession({ right: true, items: [{ kind: "page", name: "Plan", pageKind: "page" }] });
      const host = document.createElement("div"); document.body.append(host);
      const dispose = render(() => surface === "page" ? <PageView /> : surface === "sidebar" ? <RightSidebar /> : <AstBody raw="{{embed [[Plan]]}}" />, host);
      try {
        const link = await vi.waitFor(() => { const link = host.querySelector<HTMLAnchorElement>("a.file-link"); expect(link).not.toBeNull(); return link!; });
        link.click(); expect(external).toHaveBeenCalledExactlyOnceWith("file:///home/user/files/report.odt");
      } finally { dispose(); }
    });
  }
});

describe("file path classification and encoding", () => {
  it("keeps schemes, file URLs, anchors and page destinations on their existing routes", () => {
    for (const dest of ["https://x/a.txt", "zotero://x", "tel:+42", "#anchor.x", "*heading.x", "Some Page", "files/report", "file:///tmp/a%20b.txt", "file://D:\\test.txt"]) expect(fileLinkTarget(dest), dest).toBeNull();
  });
  for (const [dest, expected] of [
    ["file:../a.txt", { kind: "outside", reason: "escapes", path: "../a.txt" }],
    ["/abs/a.txt", { kind: "outside", reason: "absolute", path: "/abs/a.txt" }],
    ["/home/me/notes", { kind: "outside", reason: "absolute", path: "/home/me/notes" }],
    ["C:\\a.txt", { kind: "outside", reason: "absolute", path: "C:/a.txt" }],
    ["D:/a%20b.txt", { kind: "outside", reason: "absolute", path: "D:/a b.txt" }],
    ["..\\..\\a.txt", { kind: "outside", reason: "escapes", path: "../../a.txt" }],
  ] as const) it(`classifies ${dest}`, () => expect(fileLinkTarget(dest)).toEqual(expected));

  it("preserves parent traversal at several depths", () => {
    expect(graphRelative("../../a.txt")).toEqual({ kind: "outside", reason: "escapes", path: "../../a.txt" });
    expect(graphRelative("files/../../a.txt")).toEqual({ kind: "outside", reason: "escapes", path: "../a.txt" });
    expect(resolvedFilePath(graphRelative("../../a.txt"), "D:\\notes\\graph")).toBe("D:/a.txt");
    expect(filePathUrl("D:/a b#%.txt")).toBe("file:///D:/a%20b%23%25.txt");
    expect(filePathUrl("/home/me/a?b.txt")).toBe("file:///home/me/a%3Fb.txt");
  });

  for (const [dest, path] of [
    ["file:///home/me/a%20b.txt", "/home/me/a b.txt"],
    ["file://D:\\test.txt", "D:/test.txt"],
    ["file:///D:/notes/a%20b.txt", "D:/notes/a b.txt"],
  ]) it(`shows and opens the existing file URL ${dest}`, () => {
    expect(fileUrlTitle(dest)).toBe(path);
    const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    const link = clickLink(`[file](<${dest}>)`);
    expect(link.title).toBe(path);
    expect(link.hasAttribute("data-broken-file-link")).toBe(false);
    expect(external).toHaveBeenCalledExactlyOnceWith(dest);
  });
});
