import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { graphMeta, setGraphMeta } from "../graphSession";
import { initLocalFileSettings } from "../localFileSettings";
import * as panes from "../panes";
import * as router from "../router";
import { AstBody } from "./body";
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
  ["md", "[open](C:%2Fnotes.txt)", "external", "file:///C:/notes.txt"],
  ["md", "[open](C:%2fnotes.txt)", "external", "file:///C:/notes.txt"],
  ["md", "[open](c:%2Fnotes.txt)", "external", "file:///c:/notes.txt"],
  ["md", "[open](c:%2fnotes.txt)", "external", "file:///c:/notes.txt"],
  ["md", "[open](C:%2Fdir/notes.pdf)", "external", "file:///C:/dir/notes.pdf"],
  ["md", "[open](C:%2fdir/notes.pdf)", "external", "file:///C:/dir/notes.pdf"],
  ["md", "[open](c:%2Fdir/notes.pdf)", "external", "file:///c:/dir/notes.pdf"],
  ["md", "[open](c:%2fdir/notes.pdf)", "external", "file:///c:/dir/notes.pdf"],
  ["md", "[open](C:/folder)", "page", "C:/folder"],
  ["md", "[open](C:\\folder)", "page", "C:\\folder"],
  ["md", "[open](C:/assets/folder)", "page", "C:/assets/folder"],
  ["md", "![open](C:/folder)", "page", "C:/folder"],
  ["md", "![open](C:\\folder)", "page", "C:\\folder"],
  ["md", "![open](C:/assets/folder)", "page", "C:/assets/folder"],
  ["md", "[open](C://folder)", "external", "file:///C://folder"],
  ["org", "[[C:/folder][open]]", "external", "file:///C:///folder"],
  ["org", "[[C:\\folder][open]]", "external", "file:///C:///folder"],
  ["org", "[[C:%2Fdir/notes.pdf][open]]", "external", "file:///C:///dir/notes.pdf"],
  ["md", "[open](C:%5Cnotes.txt)", "graph", "C:\\notes.txt"],
  ["md", "[open](c:%5cnotes.txt)", "graph", "c:\\notes.txt"],
  ["md", "[open](C:%2Fnotes.pdf)", "external", "C:%2Fnotes.pdf"],
  ["md", "[open](c:%2fnotes.pdf)", "external", "c:%2fnotes.pdf"],
] as const)("keeps the Beta %s route for %s", (format, source, route, target) => {
  setGraphMeta({ ...graphMeta()!, root: "/home/test/graph" });
  const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
  const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
  const pdf = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const page = vi.spyOn(router, "openPage").mockImplementation(() => {});
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
      ...page.mock.calls.map(([name]) => ["page", name]),
    ];
    expect(opens).toEqual([[route, target]]);
  } finally { dispose(); }
});

it.each(["raw HTML", "Hiccup"] as const)("keeps Beta's opt-in local image reads for %s", async (kind) => {
  const stem = kind === "Hiccup" ? "hiccup-preview" : "html-preview";
  const sources = [
    `file:///tmp/${stem}.png`,
    `C:/${stem}.png`,
    `FILE:///tmp/${stem}.png`,
    `C:%2F${stem}.png`,
  ];
  const source = kind === "Hiccup"
    ? `[:div ${sources.map(src => `[:img {:src "${src}"}]`).join(" ")}]`
    : `<div>${sources.map(src => `<img src="${src}"/>`).join("")}</div>`;
  const preference = vi.spyOn(backend(), "getAppBool").mockResolvedValue(false);
  const read = vi.spyOn(backend(), "readLocalImage").mockResolvedValue(new Uint8Array());
  await initLocalFileSettings();
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <AstBody raw={source} />, host);
  try {
    expect(host.querySelectorAll("img")).toHaveLength(sources.length);
    expect(read).not.toHaveBeenCalled();
    preference.mockResolvedValue(true);
    await initLocalFileSettings();
    await vi.waitFor(() => expect(read.mock.calls).toEqual([
      [`/tmp/${stem}.png`],
      [`C:/${stem}.png`],
    ]));
  } finally {
    dispose();
    preference.mockResolvedValue(false);
    await initLocalFileSettings();
  }
});
