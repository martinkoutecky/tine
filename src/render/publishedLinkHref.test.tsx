import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
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
import { checkExternalLink, navigableHref } from "../linkSchemePolicy";

beforeAll(initParser);
afterEach(() => { document.body.replaceChildren(); published.on = true; });

function href(source: string): string | null {
  const host = document.createElement("div");
  document.body.append(host);
  render(() => <InlineText text={source} />, host);
  const link = host.querySelector("a.external-link");
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
  });

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
  });
});
