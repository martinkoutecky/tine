import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { graphMeta, setGraphMeta } from "../graphSession";
import * as panes from "../panes";
import { InlineText } from "./inline";
import { initParser } from "./parse";

const platform = vi.hoisted(() => ({ mobile: false }));
vi.mock("../nativeChrome", async (original) => ({
  ...await original<typeof import("../nativeChrome")>(),
  get isMobilePlatform() { return platform.mobile; },
}));

beforeAll(initParser);
afterEach(() => {
  platform.mobile = false;
  setGraphMeta(null);
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

function click(source: string, format: "md" | "org" = "md") {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text={source} format={format} />, host);
  try {
    const link = host.querySelector("a");
    expect(link, source).not.toBeNull();
    link!.click();
  } finally { dispose(); host.remove(); }
}

it("keeps the Beta attachment and folder openers", () => {
  setGraphMeta({ ...graphMeta()!, root: "/home/user/graph" });
  const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue();
  const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue();
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue();

  click("[Open](docs.v1/myassets/)");
  click("[[docs.v1/myAsSeTs//][Open]]", "org");
  click("[Open](docs.v1\\myassets\\)");
  click("[Open](../myassets/)");
  click("[Open](./AsSeTs//)");
  click("[Open](../assets/subdir//)");
  expect(graph.mock.calls.map(([path]) => path)).toEqual(["docs.v1/myassets", "docs.v1/myAsSeTs", "docs.v1/myassets"]);
  expect(external).toHaveBeenCalledExactlyOnceWith("file:///home/user/myassets");
  expect(asset.mock.calls.map(([path]) => path)).toEqual(["", "subdir"]);
});

it.each(["mobile", "desktop with unknown root"])("keeps the Beta reader fallback on %s", (environment) => {
  platform.mobile = environment === "mobile";
  if (platform.mobile) setGraphMeta({ ...graphMeta()!, root: "/home/user/graph" });
  const reader = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue();
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue();

  for (const dest of ["docs/guide.pdf", "./guide.PDF", "docs\\guide.pdf", "docs/../guide.pdf"]) {
    click(`![Guide](${dest})`);
  }
  expect(reader.mock.calls).toEqual([
    ["guide.pdf", "Guide"], ["guide.PDF", "Guide"], ["guide.pdf", "Guide"], ["guide.pdf", "Guide"],
  ]);
  expect(graph).not.toHaveBeenCalled();
  expect(external).not.toHaveBeenCalled();

  click("[Guide](docs/guide.pdf)");
  click("[[docs/guide.pdf][Guide]]", "org");
  expect(graph.mock.calls.map(([path]) => path)).toEqual(["docs/guide.pdf", "docs/guide.pdf"]);
  expect(reader).toHaveBeenCalledTimes(4);
});

it("keeps the Beta file opener for Org drives authored with ://", () => {
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue();
  const reader = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue();
  const expected: string[][] = [];
  for (const [drive, separator] of [["C", "/"], ["c", "%2F"]] as const) {
    for (const extension of ["txt", "pdf", "PDF"]) {
      const dest = `${drive}://docs${separator}note.${extension}`;
      click(`[[${dest}][Open]]`, "org");
      click(`[[${dest}]]`, "org");
      expected.push(...Array.from({ length: 2 }, () => [`file:///${drive}://docs/note.${extension}`]));
    }
  }
  expect(external.mock.calls).toEqual(expected);
  expect(reader).not.toHaveBeenCalled();
  expect(graph).not.toHaveBeenCalled();
});

it("keeps encoded colons in ordinary graph filenames", () => {
  const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue();
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue();
  for (const dest of ["report%3Aold.txt", "docs/report%3aold.txt", "c%3Areport.txt"]) {
    click(`[Open](${dest})`);
  }
  expect(graph.mock.calls.map(([path]) => path)).toEqual(["report:old.txt", "docs/report:old.txt", "c:report.txt"]);
  expect(external).not.toHaveBeenCalled();
});

it("keeps the Beta PDF asset route when a separator is written only as a URL escape", () => {
  setGraphMeta({ ...graphMeta()!, root: "/home/user/graph" });
  const reader = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue();
  click("![Guide](docs%2Fguide.pdf)");
  click("[Guide](docs%2Fguide.pdf)");
  expect(reader.mock.calls).toEqual([["guide.pdf", "Guide"], ["guide.pdf", "Guide"]]);
  expect(graph).not.toHaveBeenCalled();
});
