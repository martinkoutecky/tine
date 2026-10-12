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
import { fileLinkTarget, graphRelative } from "./relativeFileLink";

// GH #457 (UI-RELATIVE-FILE-LINK-457): a link to a file by plain path opens
// that file from the graph folder, resolved as OG does (logseq 0.10.9
// block.cljs relative-assets-path->absolute-path + config.cljs
// get-local-asset-absolute-path: strip every leading `.` and `/`, join to the
// graph folder), whichever folder the page is in. Intentional Tine difference:
// a path that leaves the graph folder (absolute, or climbing out) is shown as
// a broken link and never opened, where OG opens it. Before the fix every such
// link went to the URL opener, which refused it as an "unsupported url scheme".

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

describe("file links open the file in the graph folder, as OG resolves them (GH #457)", () => {
  for (const [pageFile, raw, opened, format] of [
    // `files/x` and `../files/x` name the same file from pages/ and journals/.
    ["pages/Plan.md", "[report](files/report.odt)", "files/report.odt", "md"],
    ["pages/Plan.md", "[report](../files/report.odt)", "files/report.odt", "md"],
    ["journals/2026_10_12.md", "[report](files/report.odt)", "files/report.odt", "md"],
    ["journals/2026_10_12.md", "[report](../files/report.odt)", "files/report.odt", "md"],
    ["pages/projects/Plan.md", "[up](../files/a.txt)", "files/a.txt", "md"],
    ["pages/Plan.md", "[deep](../../files/a.txt)", "files/a.txt", "md"],
    ["pages/Plan.md", "[here](notes.txt)", "notes.txt", "md"],
    ["pages/Plan.md", "[sub](./sub/notes.txt)", "sub/notes.txt", "md"],
    ["pages/Plan.md", "[back](files/old/../a.txt)", "files/a.txt", "md"],
    ["pages/Plan.md", "[spaced](<../files/my file.txt>)", "files/my file.txt", "md"],
    ["pages/Plan.md", "[escaped](../files/a%20b.txt)", "files/a b.txt", "md"],
    ["pages/Plan.md", "[windows](..\\files\\a.txt)", "files/a.txt", "md"],
    ["pages/Plan.md", "[scheme](file:../files/a.txt)", "files/a.txt", "md"],
    ["pages/Plan.org", "[[file:../files/a.txt][org]]", "files/a.txt", "org"],
    ["pages/Plan.md", "[paper](../docs/paper.pdf)", "docs/paper.pdf", "md"],
    ["pages/Plan.md", "[folder](../files.d/)", "files.d", "md"],
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

  // Intentional Tine difference: OG opens these (an absolute path as is, and a
  // path its join lets climb out); Tine never opens a path outside the graph
  // folder, because a link in a note is untrusted content.
  for (const raw of ["[out](files/../../outside.txt)", "[out](../files/../../../etc/passwd.txt)", "[abs](/home/me/report.odt)", "[drive](C:\\Users\\me\\report.odt)", "[abs](/home/me/paper.pdf)"]) {
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

describe("what counts as a file link", () => {
  it("leaves schemes, anchors and page-like dests alone", () => {
    for (const dest of ["https://x/a.txt", "zotero://x", "tel:+42", "#anchor.x", "*heading.x", "Some Page", "files/report", "file:///tmp/a.txt"]) {
      expect(fileLinkTarget(dest), dest).toBeNull();
    }
    expect(fileLinkTarget("file:../a.txt")).toEqual({ kind: "graph", path: "a.txt" });
    expect(fileLinkTarget("/abs/a.txt")).toEqual({ kind: "outside", reason: "absolute" });
    expect(fileLinkTarget("C:\\a.txt")).toEqual({ kind: "outside", reason: "absolute" });
  });

  it("strips leading dots and slashes as OG does, and never climbs above the graph folder", () => {
    expect(graphRelative("../../a.txt")).toEqual({ kind: "graph", path: "a.txt" });
    expect(graphRelative("files/../../a.txt")).toEqual({ kind: "outside", reason: "escapes" });
    expect(graphRelative("..")).toEqual({ kind: "graph", path: "" });
  });
});
