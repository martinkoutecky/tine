import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { graphMeta, setGraphMeta } from "../graphSession";
import * as panes from "../panes";
import { InlineText } from "./inline";
import { initParser } from "./parse";

beforeAll(initParser);
afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
  setGraphMeta(null);
});

it.each([
  ["md", "[open](./assets/)", "asset", ""],
  ["md", "[open](../assets/manual.txt)", "asset", "manual.txt"],
  ["md", "[open](assets/nested/a%2520b.txt)", "asset", "nested/a%20b.txt"],
  ["md", "[open](assets/nested/a%20b.txt)", "asset", "nested/a b.txt"],
  ["md", "[open](assets/one%20bad%zz.txt)", "asset", "one%20bad%zz.txt"],
  ["md", "[open](ASSETS/a.txt)", "asset", "a.txt"],
  ["md", "[open](assets/nested/a%20b.pdf)", "pdf", "nested/a b.pdf"],
  ["md", "![open](../assets/nested/a%2520b.pdf)", "pdf", "nested/a%20b.pdf"],
  ["md", "[open](loose.pdf)", "pdf", "loose.pdf"],
  ["md", "[open](docs/a%20b.txt)", "graph", "docs/a b.txt"],
  ["md", "[open](docs/old/../a.pdf)", "graph", "docs/a.pdf"],
  ["md", "[open](../docs/a%20b.txt)", "external", "file:///home/test/docs/a%20b.txt"],
  ["md", "![open](../docs/a%20b.pdf)", "external-pdf", "file:///home/test/docs/a%20b.pdf"],
  ["md", "[open](#a.b)", "external", "#a.b"],
  ["md", "[open](*Heading)", "external", "*Heading"],
  ["org", "[[../assets/manual.txt][open]]", "asset", "manual.txt"],
  ["org", "[[docs/a%20b.txt][open]]", "graph", "docs/a b.txt"],
  ["org", "[[../docs/a%20b.pdf][open]]", "external-pdf", "file:///home/test/docs/a%20b.pdf"],
] as const)("keeps the Beta %s route for %s", (format, source, route, target) => {
  setGraphMeta({ ...graphMeta()!, root: "/home/test/graph" });
  const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
  const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
  const pdf = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text={source} format={format} />, host);
  try {
    host.querySelector<HTMLAnchorElement>("a")!.click();
    const opens = [
      ...asset.mock.calls.map(([path]) => ["asset", path]),
      ...graph.mock.calls.map(([path]) => ["graph", path]),
      ...external.mock.calls.map(([path]) => ["external", path]),
      ...pdf.mock.calls.map(([path, , , , options]) => [options?.externalRead ? "external-pdf" : "pdf", path]),
    ];
    expect(opens).toEqual([[route, target]]);
  } finally { dispose(); }
});
