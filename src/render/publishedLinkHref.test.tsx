import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { AudioOverlay } from "../components/AudioOverlay";
import { ExternalLink } from "../components/ExternalLink";
import { LinkSchemePrompt } from "../components/LinkSchemePrompt";
import { initLinkSchemeSettings } from "../linkSchemeSettings";
import * as panes from "../panes";
import { setToasts } from "../toasts";
import { setAudioPlayer } from "../ui";
import { AstBody } from "./body";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { renderSanitizedHtml } from "./rawHtml";

const mode = vi.hoisted(() => ({ published: true, quickShare: false }));
vi.mock("../publishedBackend", async (original) => ({
  ...(await original<typeof import("../publishedBackend")>()),
  isPublishedExport: () => mode.published,
  isQuickShareViewer: () => mode.quickShare,
}));

beforeAll(initParser);
const disposers: (() => void)[] = [];
afterEach(() => {
  disposers.splice(0).forEach((dispose) => dispose());
  vi.restoreAllMocks();
  document.body.replaceChildren();
  setAudioPlayer(null);
  setToasts([]);
});

function mount(view: () => ReturnType<typeof InlineText>) {
  const host = document.createElement("div");
  document.body.append(host);
  disposers.push(render(view, host));
  return host;
}

function activate(link: Element, how: "click" | "enter") {
  const init = { bubbles: true, cancelable: true };
  link.dispatchEvent(how === "click" ? new MouseEvent("click", init)
    : new KeyboardEvent("keydown", { ...init, key: "Enter" }));
}

async function confirmLink(link: Element, how: "click" | "enter") {
  expect(link.getAttribute("href")).toBeNull();
  expect(link.getAttribute("tabindex")).toBe("0");
  const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
  activate(link, how);
  const prompt = await vi.waitFor(() => {
    const dialog = document.querySelector<HTMLFormElement>('form[role="dialog"]');
    expect(dialog).not.toBeNull();
    return dialog!;
  });
  expect(open).not.toHaveBeenCalled();
  prompt.requestSubmit();
  await vi.waitFor(() => expect(open).toHaveBeenCalledTimes(1));
  return open.mock.calls[0];
}

describe.each([
  { name: "desktop", published: false, quickShare: false },
  { name: "published", published: true, quickShare: false },
  { name: "quick share", published: false, quickShare: true },
])("$name links", (surface) => {
  beforeEach(async () => {
    mode.published = surface.published;
    mode.quickShare = surface.quickShare;
    vi.spyOn(backend(), "getAppString").mockResolvedValue("[]");
    await initLinkSchemeSettings();
    mount(() => <LinkSchemePrompt />);
  });

  it.each([
    ["md", "[manual](foo:../assets/manual.pdf)", "foo:../assets/manual.pdf", "click"],
    ["md", "![photo](FOO:assets/photo.png)", "FOO:assets/photo.png", "enter"],
    ["md", "![voice](foo:assets/voice.mp3)", "foo:assets/voice.mp3", "click"],
    ["md", "![clip](foo:assets/clip.mp4)", "foo:assets/clip.mp4", "enter"],
    ["md", "[manual](fo\to:docs/manual.txt)", "fo\to:docs/manual.txt", "enter"],
    ["org", "[[foo:assets/manual.pdf][manual]]", "foo://assets/manual.pdf", "click"],
    ["org", "[[foo:assets/photo.png]]", "foo:assets/photo.png", "enter"],
  ] as const)("checks schemes before local rendering for %s %s", async (format, source, dest, how) => {
    const pdf = vi.spyOn(panes, "openPdf").mockReturnValue(null);
    const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
    const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const read = vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    const stream = vi.spyOn(backend(), "streamAsset").mockResolvedValue("asset-stream");
    const host = mount(() => <InlineText text={source} format={format} />);
    const link = host.querySelector("a.external-link");
    expect(link).not.toBeNull();
    expect(await confirmLink(link!, how)).toEqual([dest, "foo"]);
    for (const opener of [pdf, graph, asset, read, stream]) expect(opener).not.toHaveBeenCalled();
  });

  it.each([
    ["md", "[manual]( JaVaScRiPt:assets/refused.pdf)"],
    ["md", "[manual](data\t:docs/refused.txt)"],
    ["md", "[manual](blob:assets/refused.pdf)"],
    ["md", "![photo](data:assets/refused.png)"],
    ["org", "[[java\nscript:docs/refused.txt][manual]]"],
    ["org", "[[vb\rscript:docs/refused.txt][manual]]"],
    ["org", "[[ab\r\nout:docs/refused.txt][manual]]"],
  ] as const)("refuses %s %j without reaching a renderer opener", (format, source) => {
    const pdf = vi.spyOn(panes, "openPdf").mockReturnValue(null);
    const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
    const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const host = mount(() => format === "org" ? <AstBody raw={`intro\n${source}`} format="org" />
      : <InlineText text={source} />);
    const link = host.querySelector("a.external-link");
    expect(link).not.toBeNull();
    expect(link!.getAttribute("href")).toBeNull();
    activate(link!, "click");
    activate(link!, "enter");
    for (const opener of [pdf, graph, open, asset]) expect(opener).not.toHaveBeenCalled();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });

  it("raw HTML delegates a custom scheme click to consent", async () => {
    const host = mount(() => renderSanitizedHtml('<a href="tel:123"><b>call</b></a>'));
    expect(await confirmLink(host.querySelector("a")!, "click")).toEqual(["tel:123", "tel"]);
  });

  it("Hiccup delegates Enter to consent through the same renderer", async () => {
    const host = mount(() => <AstBody raw={'[:a {:href "tel:123"} "call"]'} />);
    expect(await confirmLink(host.querySelector("a")!, "enter")).toEqual(["tel:123", "tel"]);
  });

  it("raw HTML and Hiccup keep web and fragment hrefs, defer consent, and drop refused schemes", () => {
    const anchors = mount(() => <>
      {renderSanitizedHtml('<a href="about:blank">blocked</a><a href="https://example.org">web</a><a href="tel:123">call</a>')}
      <AstBody raw={'[:span [:a {:href "blob:https://example.org/id"} "blocked"] [:a {:href "#top"} "internal"]]'} />
    </>).querySelectorAll("a");
    expect([...anchors].map((anchor) => anchor.getAttribute("href"))).toEqual([null, "https://example.org", null, null, "#top"]);
    expect([...anchors].map((anchor) => anchor.getAttribute("data-tine-href"))).toEqual([null, null, "tel:123", null, null]);
  });

  it("ExternalLink defers custom schemes and strips refused destinations", async () => {
    const [refused, custom] = mount(() => <>
      <ExternalLink dest="about:blank">blocked</ExternalLink>
      <ExternalLink dest="zotero://select/items/A">open</ExternalLink>
    </>).querySelectorAll("a");
    expect(refused.getAttribute("href")).toBeNull();
    expect(await confirmLink(custom, "enter")).toEqual(["zotero://select/items/A", "zotero"]);
  });
});

it("file and drive links use their Beta click routes without a browser href", () => {
  mode.quickShare = false;
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
  const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
  const links = mount(() => <>
    <InlineText text="[outside](file:///tmp/note.txt)" />
    <InlineText text="[inside](file:docs/note.txt)" />
    <InlineText text="[drive](D:/notes/note.txt)" />
  </>).querySelectorAll("a");
  for (const link of links) {
    expect(link.getAttribute("href")).toBeNull();
    activate(link, "click");
  }
  const file = { purpose: "link", bindingGeneration: backend().graphBindingGeneration(), show: undefined };
  expect(external.mock.calls).toEqual([["file:///tmp/note.txt", undefined, file], ["file:///D:/notes/note.txt", undefined, file]]);
  expect(graph).toHaveBeenCalledExactlyOnceWith("docs/note.txt", backend().graphBindingGeneration(), "link");
});

it("expanded web audio uses the same remote source as inline playback", async () => {
  mode.quickShare = false;
  const stream = vi.spyOn(backend(), "streamAsset").mockResolvedValue("asset-stream");
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  const url = "HTTPS://example.org/assets/voice.mp3";
  const host = mount(() => <><AstBody raw={`![voice](${url})`} /><AudioOverlay /></>);
  const expand = host.querySelector<HTMLButtonElement>(".media-audio-widen");
  expect(expand).not.toBeNull();
  expand!.click();
  await vi.waitFor(() => expect(host.querySelectorAll("audio")).toHaveLength(2));
  expect(stream).not.toHaveBeenCalled();
  expect([...host.querySelectorAll("audio")].map((audio) => audio.getAttribute("src"))).toEqual([url, url]);
});

it("an image macro and audio overlay cannot resolve a custom scheme as an attachment", () => {
  mode.quickShare = false;
  const read = vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
  const stream = vi.spyOn(backend(), "streamAsset").mockResolvedValue("asset-stream");
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  setAudioPlayer({ url: "foo:assets/voice.mp3", name: "voice" });
  const host = mount(() => <><AstBody raw="{{img foo:assets/photo.png}}" /><AudioOverlay /></>);
  expect(host.querySelector("img")).toBeNull();
  expect.soft(read).not.toHaveBeenCalled();
  expect.soft(stream).not.toHaveBeenCalled();
});
