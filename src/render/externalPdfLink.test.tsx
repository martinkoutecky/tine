import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";
import * as panes from "../panes";
import { ExternalPdfLink } from "./ExternalPdfLink";
import { graphMeta, setGraphMeta } from "../graphSession";

beforeAll(initParser);
afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); setGraphMeta(null); });

it.each([
  ["![paper](file:///tmp/a%20paper.pdf)", "file:///tmp/a%20paper.pdf", "md"],
  ["[[file:///tmp/a%20paper.pdf][paper]]", "file:///tmp/a%20paper.pdf", "org"],
  ["![paper](file:///C:/papers/文.pdf)", "file:///C:/papers/文.pdf", "md"],
  ["![paper](file://server/share/paper.pdf)", "file://server/share/paper.pdf", "md"],
] as const)("opens %s in the reader only after a click", (source, url, format) => {
  const open = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue();
  const read = vi.spyOn(backend(), "readAsset");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <InlineText text={source} format={format} />, host);
  try {
    expect(open).not.toHaveBeenCalled(); expect(read).not.toHaveBeenCalled();
    const link = host.querySelector<HTMLAnchorElement>("a.pdf-link");
    expect(link).not.toBeNull();
    link!.click();
    expect(open).toHaveBeenCalledWith(url, "paper", undefined, undefined,
      expect.objectContaining({ externalRead: expect.any(Function) }));
    expect(external).not.toHaveBeenCalled(); expect(read).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it("an injected HTML PDF link cannot enter the native reader", () => {
  const open = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <InlineText text={'<a class="pdf-link" data-external-pdf="file:///tmp/injected.pdf" href="file:///tmp/injected.pdf">fake</a>'} />, host);
  try {
    host.querySelector("a")?.click();
    expect(open).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it("reads the link's click-time URL even if its rendered destination changes", async () => {
  const [url, setUrl] = createSignal("file:///tmp/first.pdf");
  const open = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const api = backend(), original = api.readExternalPdf;
  const read = vi.fn().mockResolvedValue(new Uint8Array([1]));
  api.readExternalPdf = read;
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <ExternalPdfLink dest={url()} path="/tmp/first.pdf" label="paper" />, host);
  try {
    expect(read).not.toHaveBeenCalled();
    host.querySelector<HTMLAnchorElement>("a")!.click();
    setUrl("file:///tmp/second.pdf");
    await open.mock.calls[0][4]!.externalRead!();
    expect(read).toHaveBeenCalledExactlyOnceWith("file:///tmp/first.pdf", api.graphBindingGeneration());
  } finally { dispose(); if (original) api.readExternalPdf = original; else delete api.readExternalPdf; }
});

// GH #732 follow-up: relative file links to PDFs open in the same read-only reader.
// The graph root is the one the file-link resolver uses (relativeFileLink.ts).
const root = "/home/user/graph";
function mountLink(source: string, format: "md" | "org") {
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <InlineText text={source} format={format} />, host);
  return { host, dispose };
}

it.each([
  ["![paper](file:../papers/a%20b.pdf)", "md", "file:///home/user/papers/a%20b.pdf"],
  ["![paper](../papers/a b.pdf)", "md", "file:///home/user/papers/a%20b.pdf"],
  ["![paper](<../papers/a b.pdf>)", "md", "file:///home/user/papers/a%20b.pdf"],
  ["![paper](..\\papers\\a%20b.pdf)", "md", "file:///home/user/papers/a%20b.pdf"],
  ["![paper](./docs/a.pdf)", "md", "file:///home/user/graph/docs/a.pdf"],
  ["![paper](docs/a.pdf)", "md", "file:///home/user/graph/docs/a.pdf"],
  ["![paper](file:docs/a.pdf)", "md", "file:///home/user/graph/docs/a.pdf"],
  ["![paper](file:a.pdf)", "md", "file:///home/user/graph/a.pdf"],
  ["[[file:../papers/a%20b.pdf][paper]]", "org", "file:///home/user/papers/a%20b.pdf"],
  ["[[file:docs/a.pdf][paper]]", "org", "file:///home/user/graph/docs/a.pdf"],
] as const)("opens the relative PDF link %s in the reader, reading the resolved file", async (source, format, url) => {
  setGraphMeta({ ...graphMeta()!, root });
  const open = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue();
  const graphFile = vi.spyOn(backend(), "openGraphFile").mockResolvedValue();
  const asset = vi.spyOn(backend(), "readAsset");
  const api = backend(), original = api.readExternalPdf;
  const read = vi.fn().mockResolvedValue(new Uint8Array([1]));
  api.readExternalPdf = read;
  const { host, dispose } = mountLink(source, format);
  try {
    const link = host.querySelector<HTMLAnchorElement>("a.pdf-link");
    expect(link).not.toBeNull();
    expect(link!.title).toBe(decodeURIComponent(new URL(url).pathname));
    expect(open).not.toHaveBeenCalled(); expect(read).not.toHaveBeenCalled();
    link!.click();
    expect(open).toHaveBeenCalledWith(url, "paper", undefined, undefined,
      expect.objectContaining({ externalRead: expect.any(Function) }));
    await open.mock.calls[0][4]!.externalRead!();
    expect(read).toHaveBeenCalledExactlyOnceWith(url, api.graphBindingGeneration());
    for (const spy of [external, graphFile, asset]) expect(spy).not.toHaveBeenCalled();
  } finally { dispose(); if (original) api.readExternalPdf = original; else delete api.readExternalPdf; }
});

it("two spellings of one relative PDF link name one file, so they share one reader tab", () => {
  setGraphMeta({ ...graphMeta()!, root });
  const open = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const { host, dispose } = mountLink("![a](../papers/x.pdf) ![b](file:../papers/x.pdf) ![c](file:///home/user/papers/x.pdf)", "md");
  try {
    for (const link of host.querySelectorAll<HTMLAnchorElement>("a.pdf-link")) link.click();
    expect(open.mock.calls.map(call => call[0])).toEqual(Array(3).fill("file:///home/user/papers/x.pdf"));
  } finally { dispose(); }
});

it.each([
  ["[paper](../papers/a.pdf)", "md", "external", "file:///home/user/papers/a.pdf"],
  ["[paper](docs/a.pdf)", "md", "graph", "docs/a.pdf"],
  ["[notes](../papers/a.txt)", "md", "external", "file:///home/user/papers/a.txt"],
  ["[notes](docs/a.txt)", "md", "graph", "docs/a.txt"],
  ["[[file:../papers/a.txt][notes]]", "org", "external", "file:///home/user/papers/a.txt"],
  ["![paper](/home/me/a.pdf)", "md", "external", "file:///home/me/a.pdf"],
  // Resolving into the graph's assets/ keeps whatever the spelling did before: the file: forms bypass the asset route.
  ["![paper](file:assets/a.pdf)", "md", "graph", "assets/a.pdf"],
  ["![paper](file:docs/../Assets/a.pdf)", "md", "graph", "Assets/a.pdf"],
  ["[[file:./assets/sub/a.pdf][paper]]", "org", "graph", "assets/sub/a.pdf"],
  ["![paper](file:../graph/assets/a.pdf)", "md", "external", "file:///home/user/graph/assets/a.pdf"],
] as const)("%s keeps its existing opener, not the reader", (source, format, opener, target) => {
  setGraphMeta({ ...graphMeta()!, root });
  const open = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue();
  const graphFile = vi.spyOn(backend(), "openGraphFile").mockResolvedValue();
  const { host, dispose } = mountLink(source, format);
  try {
    expect(host.querySelector("a.pdf-link")).toBeNull();
    host.querySelector<HTMLAnchorElement>("a")!.click();
    if (opener === "external") expect(external).toHaveBeenCalledExactlyOnceWith(target);
    else expect(graphFile).toHaveBeenCalledExactlyOnceWith(target, backend().graphBindingGeneration());
    expect(open).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it.each(["![paper](../assets/a.pdf)", "![paper](assets/a.pdf)", "![paper](a.pdf)"])(
  "%s keeps the graph-asset reader with annotations", (source) => {
    setGraphMeta({ ...graphMeta()!, root });
    const open = vi.spyOn(panes, "openPdf").mockReturnValue(null);
    const { host, dispose } = mountLink(source, "md");
    try {
      host.querySelector<HTMLAnchorElement>("a.pdf-link")!.click();
      expect(open).toHaveBeenCalledExactlyOnceWith("a.pdf", "paper");
    } finally { dispose(); }
  });

it("without a graph root a relative PDF link keeps the file-link opener", () => {
  const open = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const { host, dispose } = mountLink("![paper](../papers/a.pdf)", "md");
  try {
    expect(host.querySelector("a.pdf-link")).toBeNull();
    expect(host.querySelector("a.file-link")).not.toBeNull();
    expect(open).not.toHaveBeenCalled();
  } finally { dispose(); }
});
