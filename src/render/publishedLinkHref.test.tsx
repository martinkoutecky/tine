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
import { checkExternalLink, navigableHref } from "../linkSchemePolicy";
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

const anchors = (html: string, isPublished: boolean) => {
  const root = document.createElement("div");
  root.innerHTML = rawHtmlPresentation(html, false, isPublished).html;
  return [...root.querySelectorAll("a")].map((a) => a.getAttribute("href"));
};

describe("published links keep an href only for direct schemes (GH #984)", () => {
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
      expect(await activateAndConfirm(link, how)).toEqual([["zotero://select/items/A", "zotero"]]);
    });

    it(`a published raw-HTML tel: anchor opens through the prompt (${how})`, async () => {
      const anchor = mount(() => renderSanitizedHtml('<a href="tel:123"><b>call</b></a>')).querySelector("a")!;
      expect(anchor.getAttribute("href")).toBeNull();
      expect(anchor.getAttribute("tabindex")).toBe("0");
      expect(await activateAndConfirm(how === "click" ? anchor.querySelector("b")! : anchor, how)).toEqual([["tel:123", "tel"]]);
    });
  }

  it("the app keeps custom-scheme hrefs but never a refused one", () => {
    published.on = false;
    expect(href("[z](zotero://select/items/A)")).toBe("zotero://select/items/A");
    expect(href("[b](blob:https://example.org/1)")).toBeNull();
  });

  it("raw HTML and Hiccup anchors", () => {
    const html = '<a href="https://example.org">w</a><a href="tel:123">t</a><a href="sms:1">s</a>'
      + '<a href="matrix:u/a">x</a><a href="#top">i</a><a href="about:blank">a</a>';
    expect(anchors(html, true)).toEqual(["https://example.org", null, null, null, "#top", null]);
    expect(anchors(html, false)).toEqual(["https://example.org", "tel:123", "sms:1", "matrix:u/a", "#top", null]);
  });

  it("obfuscated refused schemes lose the href in every mode", () => {
    for (const mode of [true, false]) {
      for (const dest of [" javascript:alert(1)", "java\tscript:alert(1)", "JAVASCRIPT:alert(1)", "vbscript:x"]) {
        expect(navigableHref(dest, mode), `${mode} ${JSON.stringify(dest)}`).toBe(false);
      }
    }
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
