import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";
import * as panes from "../panes";
import { ExternalPdfLink } from "./ExternalPdfLink";

beforeAll(initParser);
afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); });

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
