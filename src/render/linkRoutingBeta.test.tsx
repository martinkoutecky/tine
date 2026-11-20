import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import attachmentCases from "../../tests/regressions/gh984-attachments.json";
import { clearAssetBlobCache } from "../assetCache";
import { backend } from "../backend";
import { graphMeta, setGraphMeta } from "../graphSession";
import { anchorDecision } from "../linkSchemePolicy";
import * as panes from "../panes";
import { AstBody } from "./body";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { renderSanitizedHtml } from "./rawHtml";

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
  clearAssetBlobCache();
  document.body.replaceChildren();
});

function click(source: string, format: "md" | "org" = "md") {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text={source} format={format} />, host);
  try {
    const link = host.querySelector("a");
    expect.soft(link, source).not.toBeNull();
    link?.click();
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

it("uses the same written attachment names as trash and publication (GH #984)", async () => {
  const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue();
  const read = vi.spyOn(backend(), "readAsset").mockRejectedValue("not-found");
  const pdf = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue();
  setGraphMeta({ ...graphMeta()!, root: "/home/user/graph" });
  const host = document.createElement("div");
  document.body.append(host);

  const observe = async (source: string, format: "md" | "org") => {
    const dispose = render(() => <InlineText text={source} format={format} />, host);
    try {
      host.querySelector("a")?.click();
      await Promise.resolve();
      return [asset, read, pdf, external].map((route) => route.mock.calls.map(([name]) => name));
    } finally {
      dispose();
      for (const route of [asset, read, pdf, external]) route.mockClear();
      clearAssetBlobCache();
    }
  };

  for (const { format, source, child, name, reader } of attachmentCases.forms) {
    for (let mask = 0; mask < 64; mask++) {
      const folder = [..."assets"].map((letter, i) => mask & (1 << i) ? letter.toUpperCase() : letter).join("");
      const link = source.replace("$dest", `../${folder}/${child}`);
      const expected = ["open", "image", "pdf"].map((route) => route === reader ? [name] : []);
      expect.soft((await observe(link, format as "md" | "org")).slice(0, 3), link).toEqual(expected);
    }
    for (const marker of ["%61ssets/", "assets%2F", "assets%5C", "%41ssets%5C"]) {
      const dest = `../${marker}${child}`;
      const link = source.replace("$dest", dest);
      const folder = marker.startsWith("%41") ? "Assets" : "assets";
      const outsidePdf = reader === "pdf" && (format === "org" || source.startsWith("!"))
        ? [`file:///home/user/${folder}/${name}`] : [];
      expect.soft((await observe(link, format as "md" | "org")).slice(0, 3), link).toEqual([[], [], outsidePdf]);
      if (reader === "pdf" || reader === "open") {
        expect.soft(anchorDecision(dest), link).toMatchObject({ kind: "relative", attachment: null, target: { kind: "outside" } });
      }
    }
  }
  for (const [url, name] of attachmentCases.exactNames) {
    expect.soft((await observe(`![Open](../assets/${url})`, "md"))[1], url).toEqual([name]);
  }
  for (const [format, source, file] of attachmentCases.nonAttachments) {
    const link = source.replace("$file", file);
    const fallback = file === "guide.pdf" ? [file] : [];
    expect.soft((await observe(link, format as "md" | "org")).slice(0, 3), link).toEqual([[], [], fallback]);
  }
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

it("opens a bare Org link to an encoded drive as a file, never a page (review 1102600ff F3)", () => {
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue();
  const dests = ["C%3A//docs/note.PDF", "C%3A/docs/note.txt", "%43:%5Cdocs%5Cnote", "%43%3A%2Fdocs%2Fnote.txt"];
  for (const dest of dests) click(`[[${dest}]]`, "org");
  expect(external).toHaveBeenCalledTimes(dests.length);
  for (const [url] of external.mock.calls) expect(url).toMatch(/^file:\/\/\/C:\//);
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

function byteOptions(byte: string): Set<string> {
  const options = new Set([byte]);
  for (const literal of byte === "/" ? ["/", "\\"] : [byte]) {
    const hex = literal.charCodeAt(0).toString(16).padStart(2, "0");
    options.add(literal).add(`%${hex}`).add(`%${hex.toUpperCase()}`);
  }
  return options;
}

function byteSpellings(text: string): string[] {
  return [...new Set([...text].flatMap((byte, index) =>
    [...byteOptions(byte)].map((spelling) => text.slice(0, index) + spelling + text.slice(index + 1)),
  ))];
}

it("opens the same local target for equivalent destination bytes (GH #984)", async () => {
  platform.mobile = true;
  setGraphMeta({ ...graphMeta()!, root: "/home/user/graph" });
  const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue();
  const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue();
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue();
  const pdf = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const formats = ["md", "org"] as const;
  const source = (dest: string, format: typeof formats[number], embed = false) => {
    // In Org a final backslash would escape the closing bracket of the label.
    if (format === "org") return `[[${dest.endsWith("\\") ? dest + "\\" : dest}][Open]]`;
    // In Markdown a literal backslash must survive punctuation escaping.
    return `${embed ? "!" : ""}[Open](${dest.replaceAll("\\", "\\\\")})`;
  };
  const handoffs = () => [asset, graph, external, pdf].map((route) => {
    const calls = route.mock.calls.map(([target]) => target);
    route.mockClear();
    return calls;
  });
  const host = document.createElement("div");
  document.body.append(host);
  for (const stem of ["C:/a", "z:/B"]) {
    for (const extension of ["txt", "pdf", "PDF"]) {
      const path = `${stem}.${extension}`;
      const spellings = [...path].reduce((prefixes, byte) =>
        prefixes.flatMap((prefix) => [...byteOptions(byte)].map((option) => prefix + option)), [""]);
      for (const spelling of spellings) {
        expect(anchorDecision(spelling), spelling).toEqual({
          kind: "drive", target: { kind: "outside", reason: "absolute", path },
        });
      }
      for (const [format, embed] of [["md", false], ["md", true], ["org", false]] as const) {
        for (let i = 0; i < spellings.length; i += 64) {
          const batch = spellings.slice(i, i + 64);
          const text = batch.map((dest) => source(dest, format, embed)).join(" ");
          const dispose = render(() => <InlineText text={text} format={format} />, host);
          try {
            const links = host.querySelectorAll("a");
            expect(links).toHaveLength(batch.length);
            links.forEach((link) => link.click());
            expect.soft(handoffs(), `${format} ${batch[0]}`).toEqual([[], [], Array(batch.length).fill(`file:///${path}`), []]);
          } finally { dispose(); }
        }
      }
    }
  }
  for (const prefix of ["docs.v1/", "../docs.v1/", "/tmp/docs.v1/"]) {
    for (const suffix of ["AsSeTs/", "assets/photo.png", "assets//photo.png", "assets/sub/guide.PDF"]) {
      const dest = prefix + suffix;
      for (const format of formats) {
        click(source(dest, format), format);
        const expected = handoffs();
        expect(expected.flat()).toHaveLength(1);
        for (const spelling of byteSpellings(dest).filter((value) => /[/\\]assets(?:[/\\]|$)/i.test(value))) {
          click(source(spelling, format), format);
          expect.soft(handoffs(), `${format} ${spelling}`).toEqual(expected);
        }
      }
    }
  }
  for (const [spelling, decoded] of [
    ["docs.v1/assets/%2F", "docs.v1/assets//"],
    ["docs.v1/assets/%5C", "docs.v1/assets//"],
    ["assets/sub%2Fphoto.png", "assets/sub/photo.png"],
    ["assets/sub%5Cphoto.png", "assets/sub/photo.png"],
    ["guide.p%64f", "guide.pdf"],
    ["guide.%50DF", "guide.PDF"],
    ["guide%2Epdf", "guide.pdf"],
    ["guide%2e%50DF", "guide.PDF"],
    ["docs/guide.p%64f", "docs/guide.pdf"],
    ["docs/guide.P%44F", "docs/guide.PDF"],
    ["assets/guide.p%64f", "assets/guide.pdf"],
    ["../assets/sub/guide.P%44F", "../assets/sub/guide.PDF"],
    ["C%3A%2Fdocs%2Fbad%.txt", "C:/docs/bad%.txt"],
    ["%43:%5Cdocs%5Cbad%ZZ.PDF", "C:/docs/bad%ZZ.PDF"],
    ["C%3a%5Cassets/bad%.png", "C:/assets/bad%.png"],
    ["C%3a%5cassets/bad%.png", "C:/assets/bad%.png"],
    ["%43%3a%2fassets/bad%.mp3", "C:/assets/bad%.mp3"],
    ["C%3A/docs/bad%FF.pdf", "C:/docs/bad�.pdf"],
    ["%43%3A/docs/bad%FF.pdf", "C:/docs/bad�.pdf"],
    ["C%3A/docs/bad%2.pdf", "C:/docs/bad%2.pdf"],
    ["%2Ftmp%2Fguide.pdf", "/tmp/guide.pdf"],
    ["..%2Fguide.PDF", "../guide.PDF"],
  ]) {
    for (const format of formats) {
      click(source(decoded, format), format);
      const expected = handoffs();
      expect.soft(expected.flat(), decoded).toHaveLength(1);
      click(source(spelling, format), format);
      expect.soft(handoffs(), `${format} ${spelling}`).toEqual(expected);
    }
    if (/\.pdf$/i.test(decoded) || /^[a-z]:\//i.test(decoded)) {
      click(source(decoded, "md", true));
      const expected = handoffs();
      click(source(spelling, "md", true));
      expect.soft(handoffs(), spelling).toEqual(expected);
    }
  }

  for (const prefix of ["", "docs/", "../assets/sub/"]) {
    for (const [escape, literal] of [["%", "%"], ["%ZZ", "%ZZ"], ["%2", "%2"], ["%FF", "�"]]) {
      const dest = `${prefix}bad%20${escape}.PDF`;
      const child = prefix.includes("assets") ? "sub/" : "";
      click(source(dest, "md", true));
      expect.soft(handoffs(), dest).toEqual([[], [], [], [`${child}bad ${literal}.PDF`]]);
      click(source(dest, "org"), "org");
      expect.soft(handoffs(), dest).toEqual(prefix === "docs/"
        ? [[], [`docs/bad ${literal}.PDF`], [], []]
        : [[], [], [], [`${child}bad ${literal}.PDF`]]);
    }
  }
  for (const [dest, expected] of [
    ["bad%20%.pdf", [[], [], [], ["bad %.pdf"]]],
    ["bad%20%G0.PDF", [[], [], [], ["bad %G0.PDF"]]],
    ["assets/sub%2Fbad%20%ZZ.pdf", [[], [], [], ["sub/bad %ZZ.pdf"]]],
    ["docs/bad%20%E9.PDF", [[], [], [], ["bad �.PDF"]]],
    ["docs/bad%20%2.pdf", [[], [], [], ["bad %2.pdf"]]],
    ["docs/bad%20%25.pdf", [[], [], [], ["bad %.pdf"]]],
  ] as const) {
    click(source(dest, "md", true));
    expect.soft(handoffs(), dest).toEqual(expected);
  }
  for (const [dest, expected] of [
    ["docs.v1/%61ssets/", [[], ["docs.v1/assets"], [], []]],
    ["docs.v1/assets%2F", [[], ["docs.v1/assets"], [], []]],
    ["docs.v1/ASSET%53%5c", [[], ["docs.v1/ASSETS"], [], []]],
    ["../%61ssets/", [[], [], ["file:///home/user/assets"], []]],
    ["/docs.v1/assets%2F", [[], [], ["file:///docs.v1/assets/"], []]],
    ["docs.v1/%41ssets/photo.png", [[], ["docs.v1/Assets/photo.png"], [], []]],
    ["docs.v1/assets%2fphoto.png", [[], ["docs.v1/assets/photo.png"], [], []]],
    ["%61ssets/photo.png", [[], ["assets/photo.png"], [], []]],
    ["assets%2Fphoto.png", [[], ["assets/photo.png"], [], []]],
    ["docs.v1/assets%252F", [[], ["docs.v1/assets%2F"], [], []]],
    ["docs.v1/assets/photo.png%20", [["photo.png "], [], [], []]],
    ["assets/sub%20folder/photo.png", [["sub folder/photo.png"], [], [], []]],
    ["C%3A%252Fassets%2Fimage.png", [[], ["C:%2Fassets/image.png"], [], []]],
  ] as const) {
    for (const format of formats) {
      click(source(dest, format), format);
      expect.soft(handoffs(), `${format} ${dest}`).toEqual(expected);
    }
  }
  // The independent HTML sanitizer strips hrefs with a literal drive colon, so
  // every encoded spelling of one is inert too (review 1102600ff F2).
  for (const dest of new Set(byteSpellings("C:/docs/note.PDF").map((path) => path.replace(":", "%3A")))) {
    const dispose = render(() => renderSanitizedHtml(`<a href="${dest}">Open</a>`), host);
    try {
      host.querySelector("a")?.click();
      expect.soft(handoffs(), dest).toEqual([[], [], [], []]);
    } finally { dispose(); }
  }
  const read = vi.spyOn(backend(), "readAsset").mockRejectedValue("not-found");
  const stream = vi.spyOn(backend(), "streamAsset").mockResolvedValue("stream");
  for (const [path, method] of [["photo.png", read], ["audio.mp3", stream]] as const) {
    for (const dest of byteSpellings(`../assets/${path}`)) {
      for (const view of [
        () => <InlineText text={source(dest, "md", true)} />,
        ...(path.endsWith(".png") ? formats.map((format) => () => <AstBody raw={`{{img ${dest}}}`} format={format} />) : []),
      ]) {
        const dispose = render(view, host);
        try {
          // Drain the mocked reads and their cache lease callbacks between mounts.
          await new Promise((resolve) => setTimeout(resolve, 0));
          const expected = /[/\\]assets[/\\]/i.test(dest) ? [path] : [];
          expect.soft(method.mock.calls.map(([target]) => target), dest).toEqual(expected);
        } finally {
          dispose();
          method.mockClear();
          clearAssetBlobCache();
        }
      }
    }
  }
  platform.mobile = false;
  setGraphMeta(null);
  click(source("docs/bad%20%.pdf", "md", true));
  expect(handoffs()).toEqual([[], [], [], ["bad %.pdf"]]);
});
