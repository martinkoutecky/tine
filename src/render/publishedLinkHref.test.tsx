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
import { initParser } from "./parse";
import { rawHtmlPresentation } from "./htmlSanitize";
import { renderSanitizedHtml } from "./rawHtml";
import { anchorDecision, checkExternalLink, rememberableLinkScheme } from "../linkSchemePolicy";
import { backend } from "../backend";
import { setToasts } from "../toasts";
import { LinkSchemePrompt } from "../components/LinkSchemePrompt";
import { initLinkSchemeSettings } from "../linkSchemeSettings";

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
  setToasts([]);
  published.on = true;
});
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function mount(view: () => ReturnType<typeof InlineText>): HTMLElement {
  const host = document.createElement("div");
  document.body.append(host);
  disposers.push(render(view, host));
  return host;
}

/** Activate the link (click or Enter), confirm the prompt, return what reached the OS opener. */
async function activateAndConfirm(link: Element, how: "click" | "enter"): Promise<unknown[][]> {
  const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
  link.dispatchEvent(how === "click"
    ? new MouseEvent("click", { bubbles: true, cancelable: true })
    : new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
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
      if (!link) continue;
      link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
      link.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
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
