import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";
import * as panes from "../panes";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { PageView } from "../components/Page";
import { setToasts, toasts } from "../toasts";
import { relativeFileLinkPath, resolvePageRelative } from "./relativeFileLink";

// GH #457 (UI-RELATIVE-FILE-LINK-457): a relative link to a file resolves
// against the file of the page the block lives on and opens that file; a path
// that climbs out of the graph folder is shown as a broken link and never
// opened. Before the fix every such link went to the URL opener, which refused
// it as an "unsupported url scheme".

beforeAll(initParser);
afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
  resetStore();
  setToasts([]);
});

function loadPage(id: string | undefined, raw: string, kind: "page" | "journal" = "page") {
  loadSingle({
    ...(id ? { id } : {}), name: "Plan", title: "Plan", kind, pre_block: null,
    blocks: [{ id: "rel-link-block", raw, collapsed: false, children: [] }],
  });
}

function clickLink(raw: string, format: "md" | "org" = "md"): HTMLAnchorElement {
  const host = document.createElement("div");
  document.body.append(host);
  render(() => <InlineText text={raw} blockId="rel-link-block" format={format} />, host);
  const link = host.querySelector<HTMLAnchorElement>("a.file-link");
  expect(link, `expected a relative file link for ${raw}`).not.toBeNull();
  link!.click();
  return link!;
}

describe("relative file links open the file next to the page (GH #457)", () => {
  for (const [pageFile, raw, opened, format] of [
    ["pages/Plan.md", "[report](../files/report.odt)", "files/report.odt", "md"],
    ["journals/2026_10_12.md", "[report](../files/report.odt)", "files/report.odt", "md"],
    ["pages/Plan.md", "[here](notes.txt)", "pages/notes.txt", "md"],
    ["pages/Plan.md", "[sub](./sub/notes.txt)", "pages/sub/notes.txt", "md"],
    ["pages/projects/Plan.md", "[up](../files/a.txt)", "pages/files/a.txt", "md"],
    ["pages/Plan.md", "[spaced](<../files/my file.txt>)", "files/my file.txt", "md"],
    ["pages/Plan.md", "[escaped](../files/a%20b.txt)", "files/a b.txt", "md"],
    ["pages/Plan.md", "[windows](..\\files\\a.txt)", "files/a.txt", "md"],
    ["pages/Plan.md", "[scheme](file:../files/a.txt)", "files/a.txt", "md"],
    ["pages/Plan.org", "[[file:../files/a.txt][org]]", "files/a.txt", "org"],
    ["pages/Plan.md", "[paper](../docs/paper.pdf)", "docs/paper.pdf", "md"],
    ["pages/Plan.md", "[folder](../files/)", "files", "md"],
  ] as const) {
    it(`${raw} on ${pageFile} opens ${opened}`, () => {
      loadPage(pageFile, raw);
      const open = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
      const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      const reader = vi.spyOn(panes, "openPdf").mockReturnValue(null);
      clickLink(raw, format);
      expect(open).toHaveBeenCalledExactlyOnceWith(opened, backend().graphBindingGeneration());
      expect(external).not.toHaveBeenCalled();
      expect(reader).not.toHaveBeenCalled();
    });
  }

  for (const raw of ["[out](../../outside.txt)", "[out](../files/../../../etc/passwd)"]) {
    it(`${raw} leaves the graph folder: shown broken, never opened`, () => {
      loadPage("pages/Plan.md", raw);
      const open = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
      const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      const link = clickLink(raw);
      expect(link.hasAttribute("data-broken-file-link")).toBe(true);
      expect(open).not.toHaveBeenCalled();
      expect(external).not.toHaveBeenCalled();
      expect(toasts().map((toast) => toast.message).join("\n")).toContain("outside the graph folder");
    });
  }

  it("a missing file or refused open is reported", async () => {
    loadPage("pages/Plan.md", "[gone](../files/gone.txt)");
    vi.spyOn(backend(), "openGraphFile").mockRejectedValue(new Error("files/gone.txt could not be found"));
    clickLink("[gone](../files/gone.txt)");
    await vi.waitFor(() => expect(toasts().map((toast) => toast.message).join("\n")).toContain("could not be found"));
  });

  it("a page with no file yet says so instead of guessing", () => {
    loadPage(undefined, "[x](../files/a.txt)");
    const open = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
    clickLink("[x](../files/a.txt)");
    expect(open).not.toHaveBeenCalled();
    expect(toasts().map((toast) => toast.message).join("\n")).toContain("no file yet");
  });

  it("links into assets keep the asset routes", () => {
    loadPage("pages/Plan.md", "[a](../assets/a.txt)");
    const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const open = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
    const host = document.createElement("div");
    document.body.append(host);
    render(() => <InlineText text="[a](../assets/a.txt)" blockId="rel-link-block" />, host);
    host.querySelector<HTMLAnchorElement>("a")!.click();
    expect(asset).toHaveBeenCalledExactlyOnceWith("a.txt", backend().graphBindingGeneration());
    expect(open).not.toHaveBeenCalled();
  });

  it("opens from the routed page view", async () => {
    const raw = "[report](../files/report.odt)";
    const page = {
      id: "pages/Plan.md", name: "Plan", title: "Plan", kind: "page" as const, pre_block: null,
      blocks: [{ id: "rel-link-block", raw, collapsed: false, children: [] }],
    };
    loadSingle(page);
    vi.spyOn(backend(), "getPage").mockResolvedValue(page);
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(page);
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
    const open = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
    panes.resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Plan", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <PageView />, host);
    try {
      const link = await vi.waitFor(() => {
        const found = host.querySelector<HTMLAnchorElement>("a.file-link");
        expect(found).not.toBeNull();
        return found!;
      });
      link.click();
      expect(open).toHaveBeenCalledExactlyOnceWith("files/report.odt", backend().graphBindingGeneration());
    } finally { dispose(); }
  });
});

describe("what counts as a relative file link", () => {
  it("leaves schemes, absolute paths, anchors and bare page words alone", () => {
    for (const dest of ["https://x/a.txt", "zotero://x", "tel:+42", "/abs/a.txt", "C:\\a.txt", "~/a.txt", "#anchor", "Some Page"]) {
      expect(relativeFileLinkPath(dest), dest).toBeNull();
    }
    expect(relativeFileLinkPath("file:///tmp/a.txt")).toBeNull();
    expect(relativeFileLinkPath("file:../a.txt")).toBe("../a.txt");
  });

  it("never climbs above the graph root", () => {
    expect(resolvePageRelative("pages/Plan.md", "../../a.txt")).toEqual({ kind: "outside" });
    expect(resolvePageRelative("Plan.md", "../a.txt")).toEqual({ kind: "outside" });
    expect(resolvePageRelative("pages/Plan.md", "..")).toEqual({ kind: "graph", path: "" });
  });
});
