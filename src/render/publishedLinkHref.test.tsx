import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

// GH #984 review (033c3cc8c): in a published reader, a link keeps a browser-navigable
// href only for web, mail and internal destinations; every other scheme goes through
// the checked opener. Refused schemes never keep an href anywhere.
const published = vi.hoisted(() => ({ on: true }));
vi.mock("../publishedBackend", async (original) => ({
  ...(await original<typeof import("../publishedBackend")>()),
  isPublishedExport: () => published.on,
}));

import { InlineText } from "./inline";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { rawHtmlPresentation } from "./htmlSanitize";
import { renderSanitizedHtml } from "./rawHtml";
import { anchorDecision, checkExternalLink, rememberableLinkScheme } from "../linkSchemePolicy";
import { backend } from "../backend";
import * as panes from "../panes";
import { setToasts, toasts } from "../toasts";
import { LinkSchemePrompt } from "../components/LinkSchemePrompt";
import { AudioOverlay } from "../components/AudioOverlay";
import { initLinkSchemeSettings } from "../linkSchemeSettings";
import { setAudioPlayer } from "../ui";

beforeAll(initParser);
const disposers: (() => void)[] = [];
beforeEach(async () => {
  vi.spyOn(backend(), "getAppString").mockResolvedValue("[]");
  await initLinkSchemeSettings();
  const host = document.createElement("div"); document.body.append(host);
  disposers.push(render(() => <LinkSchemePrompt />, host));
});
afterEach(() => {
  disposers.splice(0).forEach((dispose) => dispose());
  vi.restoreAllMocks();
  document.body.replaceChildren();
  setAudioPlayer(null);
  setToasts([]);
  published.on = true;
});
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const activationEvent = (how: "click" | "enter") => how === "click"
  ? new MouseEvent("click", { bubbles: true, cancelable: true })
  : new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });

function mount(view: () => ReturnType<typeof InlineText>): HTMLElement {
  const host = document.createElement("div");
  document.body.append(host);
  disposers.push(render(view, host));
  return host;
}

/** Activate the link (click or Enter), confirm the prompt, return what reached the OS opener. */
async function activateAndConfirm(link: Element, how: "click" | "enter"): Promise<unknown[][]> {
  const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
  const event = activationEvent(how);
  link.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(true);
  await flush();
  const prompt = document.querySelector<HTMLFormElement>('[role="dialog"]');
  expect(prompt, "consent prompt").not.toBeNull();
  prompt!.requestSubmit();
  await flush();
  return open.mock.calls;
}

function href(source: string): string | null {
  const link = mount(() => <InlineText text={source} />).querySelector("a.external-link");
  expect(link, source).not.toBeNull();
  return link!.getAttribute("href");
}

const anchors = (html: string) => {
  const root = document.createElement("div");
  root.innerHTML = rawHtmlPresentation(html).html;
  return [...root.querySelectorAll("a")].map((a) => a.getAttribute("href"));
};

describe("links keep an href only for relative, web and mail destinations (GH #984)", () => {
  it("Markdown links in a published reader", () => {
    expect(href("[w](https://example.org/a)")).toBe("https://example.org/a");
    expect(href("[m](mailto:a@b.c)")).toBe("mailto:a@b.c");
    expect(href("[z](zotero://select/items/A)")).toBeNull();
    expect(href("[f](file:///etc/passwd)")).toBeNull();
    expect(href("[t](tine://open?page=x)")).toBeNull();
    expect(href("[b](blob:https://example.org/1)")).toBeNull();
    expect(href("[c](c:/notes/x.txt)")).toBeNull();
    expect(href("[n](file:notes.txt)")).toBeNull();
  });

  for (const how of ["click", "enter"] as const) {
    it(`a published custom-app link stays focusable and opens through the prompt (${how})`, async () => {
      const link = mount(() => <InlineText text="[z](zotero://select/items/A)" />).querySelector("a.external-link")!;
      expect(link.getAttribute("tabindex")).toBe("0");
      expect(link.getAttribute("role")).toBe("link");
      expect(link.getAttribute("data-tine-href")).toBe("zotero://select/items/A");
      expect(await activateAndConfirm(link, how)).toEqual([["zotero://select/items/A", "zotero"]]);
    });

    it(`a published raw-HTML tel: anchor opens through the prompt (${how})`, async () => {
      const anchor = mount(() => renderSanitizedHtml('<a href="tel:123"><b>call</b></a>')).querySelector("a")!;
      expect(anchor.getAttribute("href")).toBeNull();
      expect(anchor.getAttribute("tabindex")).toBe("0");
      expect(await activateAndConfirm(how === "click" ? anchor.querySelector("b")! : anchor, how)).toEqual([["tel:123", "tel"]]);
    });
  }

  it("the app follows the same href policy", () => {
    published.on = false;
    expect(href("[w](https://example.org/a)")).toBe("https://example.org/a");
    expect(href("[z](zotero://select/items/A)")).toBeNull();
    expect(href("[b](blob:https://example.org/1)")).toBeNull();
  });

  it("relative links keep the Beta path: the checked opener, or the asset opener for attachments (review 9a34ea9dd)", async () => {
    const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    for (const source of ["[s](#section)", "[d](#a.b)", "[h](*Heading)", "[a](../assets/manual.txt)", "[b](assets/manual.txt)"]) {
      const link = mount(() => <InlineText text={source} />).querySelector("a.external-link")!;
      expect(link.getAttribute("href"), source).not.toBeNull();
      const click = new MouseEvent("click", { bubbles: true, cancelable: true });
      link.dispatchEvent(click);
      expect(click.defaultPrevented, source).toBe(true);
    }
    await flush();
    expect(open.mock.calls.map(([dest]) => dest)).toEqual(["#section", "#a.b", "*Heading"]);
    expect(asset.mock.calls.map(([rel]) => rel)).toEqual(["manual.txt", "manual.txt"]);
  });

  it("a refused scheme naming assets/ never reaches the attachment opener (review ef9423888)", async () => {
    const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    for (const dest of ["\u0001javascript:assets/manual.txt", "java\tscript:assets/manual.txt", " data:assets/manual.txt", "vbscript:../assets/manual.txt"]) {
      const link = mount(() => <InlineText text={`[x](${dest})`} />).querySelector("a.external-link");
      expect(link, JSON.stringify(dest)).not.toBeNull();
      link!.dispatchEvent(activationEvent("click"));
      link!.dispatchEvent(activationEvent("enter"));
    }
    await flush();
    expect(asset).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
  });

  it("raw HTML and Hiccup anchors", () => {
    const html = '<a href="https://example.org">w</a><a href="tel:123">t</a><a href="sms:1">s</a>'
      + '<a href="matrix:u/a">x</a><a href="#top">i</a><a href="about:blank">a</a>';
    expect(anchors(html)).toEqual(["https://example.org", null, null, null, "#top", null]);
  });

  it("obfuscated refused schemes are dropped", () => {
    for (const dest of [" javascript:alert(1)", "java\tscript:alert(1)", "JAVASCRIPT:alert(1)", "vbscript:x"]) {
      expect(anchorDecision(dest), JSON.stringify(dest)).toBe("drop");
    }
  });

  it("ftp, ws and wss keep their consent path (round-3 review)", () => {
    for (const scheme of ["ftp", "ws", "wss"]) expect(rememberableLinkScheme(scheme), scheme).toBe(true);
    for (const scheme of ["http", "file", "tine", "javascript", "zotero:", "a/b", "z"]) expect(rememberableLinkScheme(scheme), scheme).toBe(false);
  });
});

it("expanded web audio uses the same remote source as inline playback", async () => {
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

describe.each([true, false])("scheme checks before local routes (published renderer: %s)", (publishedMode) => {
  beforeEach(() => { published.on = publishedMode; });

  it.each([
    ["asset PDF", "md", "foo:assets/manual.pdf", "[manual](foo:assets/manual.pdf)", "foo", "click"],
    ["embedded PDF", "md", "foo:assets/figure.pdf", "![figure](foo:assets/figure.pdf)", "foo", "enter"],
    ["image", "md", "FOO:assets/photo.png", "![photo](FOO:assets/photo.png)", "foo", "click"],
    ["audio", "md", "foo:assets/voice.mp3", "![voice](foo:assets/voice.mp3)", "foo", "enter"],
    ["video", "md", "foo:assets/clip.mp4", "![clip](foo:assets/clip.mp4)", "foo", "click"],
    ["tab in graph file scheme", "md", "fo\to:docs/manual.txt", "[manual](fo\to:docs/manual.txt)", "foo", "enter"],
    ["Org PDF", "org", "foo://assets/manual.pdf", "[[foo:assets/manual.pdf][manual]]", "foo", "click"],
    ["Org bare media", "org", "foo:assets/photo.png", "[[foo:assets/photo.png]]", "foo", "enter"],
  ] as const)("takes %s to the checked opener", async (_name, format, dest, source, scheme, how) => {
    const pdf = vi.spyOn(panes, "openPdf").mockReturnValue(null);
    const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
    const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const read = vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    const stream = vi.spyOn(backend(), "streamAsset").mockResolvedValue("asset-stream");
    const host = mount(() => <InlineText text={source} format={format} />);
    expect(read).not.toHaveBeenCalled();
    expect(stream).not.toHaveBeenCalled();
    const link = host.querySelector("a.external-link");
    expect(link, source).not.toBeNull();
    expect(link!.getAttribute("href")).toBeNull();
    expect(await activateAndConfirm(link!, how)).toEqual([[dest, scheme]]);
    expect(pdf).not.toHaveBeenCalled();
    expect(graph).not.toHaveBeenCalled();
    expect(asset).not.toHaveBeenCalled();
  });

  it.each([
    ["mixed case and leading space", "md", "[manual]( JaVaScRiPt:assets/refused.pdf)", "click"],
    ["tab", "md", "[manual](data\t:docs/refused.txt)", "click"],
    ["LF", "org", "[[java\nscript:docs/refused.txt][manual]]", "enter"],
    ["CR", "org", "[[vb\rscript:docs/refused.txt][manual]]", "click"],
    ["CRLF", "org", "[[ab\r\nout:docs/refused.txt][manual]]", "enter"],
    ["blob PDF", "md", "[manual](blob:assets/refused.pdf)", "click"],
    ["data image", "md", "![photo](data:assets/refused.png)", "click"],
  ] as const)("refuses %s before any opener", async (_name, format, source, how) => {
    const pdf = vi.spyOn(panes, "openPdf").mockReturnValue(null);
    const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
    const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    const asset = vi.spyOn(backend(), "openAsset").mockResolvedValue(undefined);
    const link = mount(() => format === "org" ? <AstBody raw={`intro\n${source}`} format="org" />
      : <InlineText text={source} />).querySelector("a.external-link");
    expect(link, JSON.stringify(source)).not.toBeNull();
    const event = activationEvent(how);
    link!.dispatchEvent(event);
    await flush();
    expect(pdf).not.toHaveBeenCalled();
    expect(graph).not.toHaveBeenCalled();
    expect(asset).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(true);
    expect(link!.getAttribute("href")).toBeNull();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(toasts()).toHaveLength(1);
  });

  it("keeps scheme-less PDFs and graph files on their Beta routes", () => {
    const pdf = vi.spyOn(panes, "openPdf").mockReturnValue(null);
    const graph = vi.spyOn(backend(), "openGraphFile").mockResolvedValue(undefined);
    const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
    for (const source of ["[manual](assets/manual.pdf)", "[manual](docs/manual.txt)"]) {
      const link = mount(() => <InlineText text={source} />).querySelector("a.external-link");
      expect(link, source).not.toBeNull();
      link!.dispatchEvent(activationEvent("click"));
    }
    expect(pdf).toHaveBeenCalledExactlyOnceWith("manual.pdf", "manual");
    expect(graph).toHaveBeenCalledExactlyOnceWith("docs/manual.txt", backend().graphBindingGeneration());
    expect(open).not.toHaveBeenCalled();
  });
});

describe("custom-scheme confirmation matches case-insensitively (GH #984 review)", () => {
  it("a Zotero or ZOTERO confirmation admits zotero:", () => {
    for (const confirmed of ["zotero", "Zotero", "ZOTERO"]) {
      expect(() => checkExternalLink("zotero://select/items/A", confirmed, "native")).not.toThrow();
    }
    expect(() => checkExternalLink("zotero://select/items/A", "obsidian", "native")).toThrow();
    expect(() => checkExternalLink("javascript:alert(1)", "javascript", "native")).toThrow();
    // ASCII only, like the native opener: U+212A KELVIN SIGN lowercases to "k" in JavaScript.
    expect(() => checkExternalLink("keep://x", "\u212Aeep", "native")).toThrow();
  });
});
