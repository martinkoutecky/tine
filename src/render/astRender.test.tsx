import { describe, it, expect, beforeAll, afterEach, vi } from "vitest";
import { render } from "solid-js/web";
import { renderInlines, InlineText, expandTemplate, expansionIsBlockLevel } from "./inline";
import { AstBody, renderBlocks } from "./body";
import { initParser, parseBlock } from "./parse";
import { setGraphMeta } from "../ui";
import type { JSX } from "solid-js";
import type { Block, Inline } from "./ast";
import { backend } from "../backend";
import { clearAssetBlobCache } from "../assetCache";

// A few render paths reach back into the wasm parser (e.g. a properties block
// renders each value via InlineText → parseBlock). Node supports WebAssembly +
// atob, so we load the real vendored parser once — this doubles as a wasm smoke
// test (it must instantiate cleanly outside WebKitGTK too).
beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  setGraphMeta(null);
  clearAssetBlobCache();
  vi.restoreAllMocks();
});

function html(node: () => JSX.Element): string {
  const div = document.createElement("div");
  const dispose = render(() => node(), div);
  const out = div.innerHTML;
  dispose();
  return out;
}
const inl = (xs: Inline[]) => html(() => renderInlines(xs));
const blk = (xs: Block[]) => html(() => renderBlocks(xs));

function mountedIframeWrap(raw: string): { wrap: HTMLElement; dispose: () => void } {
  const div = document.createElement("div");
  const dispose = render(() => renderInlines([{ k: "inline_html", text: raw }]), div);
  const wrap = div.querySelector<HTMLElement>(".embed-iframe-wrap");
  if (!wrap) {
    dispose();
    throw new Error(`expected iframe wrapper in ${div.innerHTML}`);
  }
  return { wrap, dispose };
}

describe("renderInlines", () => {
  it("plain + emphasis", () => {
    const h = inl([
      { k: "plain", text: "a " },
      { k: "emphasis", emph: "Bold", children: [{ k: "plain", text: "b" }] },
      { k: "plain", text: " " },
      { k: "emphasis", emph: "Italic", children: [{ k: "plain", text: "c" }] },
    ]);
    expect(h).toContain("<strong>");
    expect(h).toContain("b");
    expect(h).toContain("<em>");
  });

  it("code + highlight + strike + underline", () => {
    expect(inl([{ k: "code", text: "x" }])).toContain('class="inline-code"');
    expect(inl([{ k: "emphasis", emph: "Highlight", children: [{ k: "plain", text: "h" }] }])).toContain("<mark>");
    expect(inl([{ k: "emphasis", emph: "Strike_through", children: [{ k: "plain", text: "s" }] }])).toContain("<del>");
    expect(inl([{ k: "emphasis", emph: "Underline", children: [{ k: "plain", text: "u" }] }])).toContain("<u>");
  });

  it("page ref renders brackets + name", () => {
    const h = inl([{ k: "link", url: { type: "page_ref", v: "My Page" }, full: "[[My Page]]" }]);
    expect(h).toContain('class="page-ref"');
    expect(h).toContain("My Page");
  });

  it("treats a nested page ref as one target when show-brackets is off", () => {
    // Tine's parser keeps `[[Outer [[Inner]] tail]]` as one page_ref, so there
    // is no separately rendered nested outer link whose brackets stay visible.
    setGraphMeta({ show_brackets: false } as never);
    const h = html(() => InlineText({ text: "[[Outer [[Inner]] tail]]" }) as JSX.Element);
    expect(h).not.toContain('class="bracket"');
    expect(h).toContain("Outer [[Inner]] tail");
  });

  it("page ref with alias label", () => {
    const h = inl([{ k: "link", url: { type: "page_ref", v: "Target" }, full: "[[Target][alias]]", label: [{ k: "plain", text: "alias" }] }]);
    expect(h).toContain('class="page-ref"');
    expect(h).toContain("alias");
  });

  it("renders an Org local-image Page_ref through AssetImage in Block/AstBody, RefBlocks/InlineText, and SheetGrid/AstBody", async () => {
    // Block and SheetGrid both use AstBody; RefBlocks uses InlineText. Exercise the
    // format-aware shared renderer rather than duplicating those caller surfaces.
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1, 2, 3]));
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL: vi.fn(() => "blob:org-page-ref-image"),
      revokeObjectURL: vi.fn(),
    });
    const host = document.createElement("div");
    const dispose = render(() => (
      <>
        <AstBody raw="[[../assets/visible.png]]" format="org" />
        <InlineText text="[[../assets/visible.png]]" format="org" />
        <AstBody raw="[[../assets/visible.png]]" format="org" />
      </>
    ), host);
    try {
      await vi.waitFor(() => expect(host.querySelectorAll("img.inline-image")).toHaveLength(3));
      expect(host.querySelector(".page-ref")).toBeNull();
    } finally {
      dispose();
      vi.unstubAllGlobals();
    }
  });

  it("control: Markdown local-image Page_ref remains a PageRef", () => {
    const h = html(() => <AstBody raw="[[../assets/visible.png]]" format="md" />);
    expect(h).toContain('class="page-ref"');
    expect(h).not.toContain("inline-image");
  });

  it("keeps an ordinary Org Page_ref as a PageRef", () => {
    const h = html(() => <AstBody raw="[[Some Page]]" format="org" />);
    expect(h).toContain('class="page-ref"');
    expect(h).not.toContain("inline-image");
  });

  it.each(["../assets/clip.mp4", "../assets/paper.pdf"])(
    "keeps Org Page_ref %s on the non-image route",
    (target) => {
      const h = html(() => <AstBody raw={`[[${target}]]`} format="org" />);
      expect(h).toContain('class="page-ref"');
      expect(h).not.toContain("inline-image");
    },
  );

  it("tag renders #name", () => {
    const h = inl([{ k: "tag", children: [{ k: "plain", text: "project" }] }]);
    expect(h).toContain('class="tag"');
    expect(h).toContain("#project");
  });

  it("external link", () => {
    const h = inl([{ k: "link", url: { type: "complex", protocol: "https", link: "example.com/x" }, full: "https://example.com/x" }]);
    expect(h).toContain('class="external-link"');
    expect(h).toContain("https://example.com/x");
  });

  it("image flag → inline-image-wrap (external)", () => {
    const h = inl([{ k: "link", url: { type: "complex", protocol: "https", link: "x.com/a.png" }, full: "![](…)", image: true, label: [{ k: "plain", text: "alt" }] }]);
    expect(h).toContain("inline-image");
  });

  it("bare_remote_image_url_renders_img", () => {
    const h = inl([{ k: "link", url: { type: "complex", protocol: "https", link: "host/pic.jpg" }, full: "https://host/pic.jpg", image: false }]);
    expect(h).toContain("<img");
    expect(h).not.toContain('class="external-link"');
  });

  it("bare_remote_image_url_with_query_renders_img", () => {
    const h = inl([{ k: "link", url: { type: "complex", protocol: "https", link: "host/pic.png?w=200#x" }, full: "https://host/pic.png?w=200#x", image: false }]);
    expect(h).toContain("<img");
    expect(h).not.toContain('class="external-link"');
  });

  it("bare_remote_video_url_renders_player", () => {
    const h = inl([{ k: "link", url: { type: "complex", protocol: "https", link: "host/clip.mp4" }, full: "https://host/clip.mp4", image: false }]);
    expect(h).toContain("<video");
    expect(h).not.toContain('class="external-link"');
  });

  it("bare_remote_audio_url_renders_player", () => {
    const h = inl([{ k: "link", url: { type: "complex", protocol: "https", link: "host/song.mp3" }, full: "https://host/song.mp3", image: false }]);
    expect(h).toContain("<audio");
    expect(h).not.toContain('class="external-link"');
  });

  it("labeled_media_link_stays_a_link", () => {
    const h = inl([{ k: "link", url: { type: "complex", protocol: "https", link: "host/pic.jpg" }, full: "[click](https://host/pic.jpg)", image: false, label: [{ k: "plain", text: "click" }] }]);
    expect(h).toContain('class="external-link"');
    expect(h).toContain("click");
    expect(h).not.toContain("<img");
  });

  it("markdown_image_still_renders", () => {
    const h = inl([{ k: "link", url: { type: "complex", protocol: "https", link: "host/pic.jpg" }, full: "![](https://host/pic.jpg)", image: true }]);
    expect(h).toContain("<img");
  });

  it("plain_nonmedia_link_stays_a_link", () => {
    const h = inl([{ k: "link", url: { type: "complex", protocol: "https", link: "example.com/page" }, full: "https://example.com/page", image: false }]);
    expect(h).toContain('class="external-link"');
    expect(h).not.toContain("<img");
  });

  it("releases a pending local-image lease when unmounted before the read finishes", async () => {
    let resolveRead!: (bytes: Uint8Array) => void;
    vi.spyOn(backend(), "readAsset").mockReturnValue(
      new Promise<Uint8Array>((resolve) => { resolveRead = resolve; })
    );
    const revokeObjectURL = vi.fn();
    vi.stubGlobal("URL", {
      createObjectURL: vi.fn(() => "blob:slow-image"),
      revokeObjectURL,
    });
    const host = document.createElement("div");
    const dispose = render(
      () => renderInlines([{
        k: "link",
        url: { type: "search", v: "../assets/slow.png" },
        full: "![](../assets/slow.png)",
        image: true,
      }]),
      host
    );
    dispose();
    clearAssetBlobCache();
    resolveRead(new Uint8Array([1, 2, 3]));
    await vi.waitFor(() => {
      expect(revokeObjectURL).toHaveBeenCalledWith("blob:slow-image");
    });
  });

  it("serializes whole-file audio fallback, bounds it globally, and releases bytes on unmount", async () => {
    vi.spyOn(backend(), "streamAsset").mockImplementation(async (name) => `asset://${name}`);
    const resolvers: Array<(bytes: Uint8Array) => void> = [];
    const read = vi.spyOn(backend(), "readAsset").mockImplementation(
      () => new Promise<Uint8Array>((resolve) => { resolvers.push(resolve); })
    );
    vi.stubGlobal("URL", {
      createObjectURL: vi.fn((blob: Blob) => `blob:media-${blob.size}`),
      revokeObjectURL: vi.fn(),
    });
    const host = document.createElement("div");
    const media = (name: string): Inline => ({
      k: "link",
      url: { type: "search", v: `../assets/${name}` },
      full: `![](../assets/${name})`,
      image: true,
    });
    const dispose = render(() => renderInlines([media("one.mp3"), media("two.mp3")]), host);
    await vi.waitFor(() => expect(host.querySelectorAll("audio")).toHaveLength(2));

    for (const audio of host.querySelectorAll("audio")) audio.dispatchEvent(new Event("error"));
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
    expect(read).toHaveBeenCalledWith("one.mp3", 64 * 1024 * 1024);

    resolvers.shift()!(new Uint8Array([1, 2, 3]));
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
    expect(read).toHaveBeenLastCalledWith("two.mp3", 64 * 1024 * 1024);
    resolvers.shift()!(new Uint8Array([4, 5, 6]));
    await vi.waitFor(() => expect(host.querySelectorAll('audio[src^="blob:media-"]')).toHaveLength(2));

    dispose();

    const nextHost = document.createElement("div");
    const disposeNext = render(() => renderInlines([media("three.mp3")]), nextHost);
    await vi.waitFor(() => expect(nextHost.querySelector("audio")).not.toBeNull());
    nextHost.querySelector("audio")!.dispatchEvent(new Event("error"));
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(3));
    expect(read).toHaveBeenLastCalledWith("three.mp3", 64 * 1024 * 1024);
    resolvers.shift()!(new Uint8Array([7, 8, 9]));
    disposeNext();
  });

  it("image-syntax PDF renders as a PDF link, not an image", () => {
    const h = inl([{ k: "link", url: { type: "search", v: "../assets/paper.pdf" }, full: "![](../assets/paper.pdf)", image: true }]);
    expect(h).toContain('class="external-link pdf-link"');
    expect(h).toContain("paper.pdf");
    expect(h).not.toContain("inline-image");
  });

  it("PDF links accept Windows-style backslash asset paths", () => {
    const h = inl([{ k: "link", url: { type: "search", v: "..\\assets\\paper.pdf" }, full: "[paper](..\\assets\\paper.pdf)" }]);
    expect(h).toContain('class="external-link pdf-link"');
    expect(h).toContain("paper.pdf");
  });

  it("latex inline", () => {
    expect(inl([{ k: "latex", mode: "Inline", body: "x^2" }])).toContain('class="math"');
  });

  it("org active timestamp formats date in <>", () => {
    const h = inl([{ k: "timestamp", ts: "Date", date: { date: { year: 2026, month: 6, day: 28 }, wday: "Sun", active: true } }]);
    expect(h).toContain("org-timestamp");
    expect(h).toContain("2026-06-28");
    expect(h).toContain("Sun");
  });

  it("entity renders the unicode glyph", () => {
    const h = inl([{ k: "entity", name: "Delta", latex: "\\Delta", latex_mathp: true, html: "&Delta;", ascii: "[Delta]", unicode: "Δ" }]);
    expect(h).toContain("Δ");
  });

  it("footnote ref", () => {
    expect(inl([{ k: "fnref", name: "1" }])).toContain('class="footnote-ref"');
  });

  it.each(["md", "org"] as const)("direct inline hiccup renders an element in %s", (format) => {
    const h = html(() => renderInlines(
      [{ k: "hiccup", v: '[:span.parity-hiccup "inline"]' }],
      undefined,
      true,
      false,
      format,
    ));
    expect(h).toContain('<span class="parity-hiccup">inline</span>');
    expect(h).not.toContain("[:span.parity-hiccup");
  });

  it.each(["md", "org"] as const)("invalid direct inline hiccup stays literal in %s", (format) => {
    const source = '[:span "unterminated"';
    const h = html(() => renderInlines([{ k: "hiccup", v: source }], undefined, true, false, format));
    expect(h).toContain(source);
    expect(h).not.toContain("<span>unterminated</span>");
  });

  it("uses iframe width and height from attrs or style", () => {
    const attrEmbed = mountedIframeWrap('<iframe src="https://example.com/" width="35%" height="63%"></iframe>');
    try {
      expect(attrEmbed.wrap.style.width).toBe("35%");
      expect(attrEmbed.wrap.style.height).toBe("63%");
      expect(attrEmbed.wrap.style.aspectRatio).toBe("auto");
    } finally {
      attrEmbed.dispose();
    }

    const styleEmbed = mountedIframeWrap('<iframe src="https://translate.google.com/" style="width:350px;height:630px"></iframe>');
    try {
      expect(styleEmbed.wrap.style.width).toBe("350px");
      expect(styleEmbed.wrap.style.height).toBe("630px");
      expect(styleEmbed.wrap.style.aspectRatio).toBe("auto");
    } finally {
      styleEmbed.dispose();
    }
  });

  // Catalog UI-YOUTUBE-EMBED-153-001 / og-parity youtube-embed-playback.
  // A YouTube embed iframe that loads with no referrer is rejected by YouTube's
  // player as error 153; OG (youtube.cljs:54-70) sends the app origin via
  // referrerpolicy + the allow list + ?enablejsapi=1.
  const mountIframeEl = (node: () => JSX.Element): { iframe: HTMLIFrameElement; dispose: () => void } => {
    const div = document.createElement("div");
    const dispose = render(node, div);
    const iframe = div.querySelector("iframe");
    if (!iframe) {
      dispose();
      throw new Error(`expected iframe in ${div.innerHTML}`);
    }
    return { iframe, dispose };
  };

  it("youtube macro embed sends a referrer + enablejsapi (no error 153)", () => {
    const { iframe, dispose } = mountIframeEl(() => renderInlines([{ k: "macro", name: "youtube", args: ["dQw4w9WgXcQ"] }]));
    try {
      expect(iframe.getAttribute("src")).toBe("https://www.youtube.com/embed/dQw4w9WgXcQ?enablejsapi=1");
      expect(iframe.getAttribute("referrerpolicy")).toBe("strict-origin-when-cross-origin");
      expect(iframe.getAttribute("allow")).toContain("encrypted-media");
      expect(iframe.getAttribute("allow")).toContain("picture-in-picture");
    } finally {
      dispose();
    }
  });

  it("vimeo macro embed gets the OG allow list and no referrerpolicy", () => {
    const { iframe, dispose } = mountIframeEl(() => renderInlines([{ k: "macro", name: "vimeo", args: ["123456789"] }]));
    try {
      expect(iframe.getAttribute("src")).toBe("https://player.vimeo.com/video/123456789");
      expect(iframe.getAttribute("allow")).toContain("encrypted-media");
      expect(iframe.getAttribute("allow")).not.toContain("web-share");
      expect(iframe.getAttribute("referrerpolicy")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("raw-HTML youtube iframe sends the app origin as referrer, not no-referrer", () => {
    const { wrap, dispose } = mountedIframeWrap('<iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ"></iframe>');
    try {
      expect(wrap.querySelector("iframe")!.getAttribute("referrerpolicy")).toBe("strict-origin-when-cross-origin");
    } finally {
      dispose();
    }
  });

  it("raw-HTML non-video iframe keeps no-referrer (privacy preserved)", () => {
    const { wrap, dispose } = mountedIframeWrap('<iframe src="https://example.com/widget"></iframe>');
    try {
      expect(wrap.querySelector("iframe")!.getAttribute("referrerpolicy")).toBe("no-referrer");
    } finally {
      dispose();
    }
  });
});

describe("renderBlocks", () => {
  it("bullet header renders inline title", () => {
    expect(blk([{ kind: "bullet", level: 1, inline: [{ k: "plain", text: "hello" }] }])).toContain("hello");
  });

  it("src code block", () => {
    expect(blk([{ kind: "src", lang: "rust", code: "fn main(){}" }])).toContain("code-block");
  });

  it("hr", () => {
    expect(blk([{ kind: "hr" }])).toContain("md-hr");
  });

  it("table renders cells", () => {
    const h = blk([{ kind: "table", header: [[{ k: "plain", text: "A" }], [{ k: "plain", text: "B" }]], rows: [[[{ k: "plain", text: "1" }], [{ k: "plain", text: "2" }]]], aligns: [] }]);
    expect(h).toContain("md-table");
    expect(h).toContain("A");
    expect(h).toContain("1");
  });

  it("md [!NOTE] callout: title once, body separate (no dup)", () => {
    const h = blk([
      {
        kind: "quote",
        children: [
          {
            kind: "paragraph",
            inline: [
              { k: "plain", text: "[!NOTE] Heads up" },
              { k: "break" },
              { k: "plain", text: "body line" },
            ],
          },
        ],
      },
    ]);
    expect(h).toContain("callout-note");
    expect(h).toContain("Heads up");
    expect(h).toContain("body line");
    // The title text must NOT be duplicated into the body.
    expect(h.split("Heads up").length - 1).toBe(1);
  });

  it("org custom callout", () => {
    const h = blk([{ kind: "custom", name: "TIP", children: [{ kind: "paragraph", inline: [{ k: "plain", text: "do this" }] }] }]);
    expect(h).toContain("callout-tip");
  });

  it("preserves a verse custom block's semantic wrapper and children", () => {
    const h = blk([{ kind: "custom", name: "VERSE", children: [{ kind: "paragraph", inline: [{ k: "plain", text: "a measured line" }] }] }]);
    expect(h).toContain('class="verse"');
    expect(h).toContain("a measured line");
  });

  it("gives every Org admonition an accessible, type-specific icon hook", () => {
    const icons = {
      note: "📝",
      tip: "💡",
      important: "❗",
      caution: "⚠️",
      warning: "🚨",
      pinned: "📌",
    };
    for (const [type, icon] of Object.entries(icons)) {
      const h = blk([{ kind: "custom", name: type.toUpperCase(), children: [] }]);
      expect(h).toContain(`class="admonition-icon admonition-icon-${type}"`);
      expect(h).toContain(`aria-label="${type} icon"`);
      expect(h).toContain(`alt="${icon}"`);
    }
  });

  it("preserves an unknown custom block's lowercased semantic wrapper", () => {
    const h = html(() => <AstBody raw={"#+BEGIN_FOO\nkept body\n#+END_FOO"} format="org" />);
    expect(h).toContain('class="foo"');
    expect(h).toContain("kept body");
  });

  it("keeps comment hidden, Example code, and NOTE body rendering unchanged", () => {
    expect(blk([{ kind: "comment", text: "do not render" }])).toBe("");
    const example = blk([{ kind: "example", code: "const example = true;" }]);
    expect(example).toContain('class="code-block"');
    expect(example).toContain("const example = true;");
    const note = blk([{ kind: "custom", name: "NOTE", children: [{ kind: "paragraph", inline: [{ k: "plain", text: "note body" }] }] }]);
    expect(note).toContain('class="callout-body"');
    expect(note).toContain("note body");
  });

  it("properties block filters id::, shows user props", () => {
    const h = blk([{ kind: "properties", props: [["id", "x"], ["author", "Martin"]] }]);
    expect(h).toContain("author");
    expect(h).not.toContain(">id<");
  });

  it("displayed math block", () => {
    expect(blk([{ kind: "displayed_math", text: "E=mc^2" }])).toContain("math-display");
  });

  it("drawer/comment render nothing", () => {
    expect(blk([{ kind: "comment", text: "c" }])).not.toContain("c");
  });

  it.each(["md", "org"] as const)("direct block hiccup renders an element in %s", (format) => {
    const h = html(() => renderBlocks(
      [{ kind: "hiccup", v: '[:div.parity-hiccup "block"]' }],
      undefined,
      undefined,
      false,
      format,
    ));
    expect(h).toContain('<div class="parity-hiccup">block</div>');
    expect(h).not.toContain("[:div.parity-hiccup");
  });

  it.each(["md", "org"] as const)("invalid direct block hiccup stays literal in %s", (format) => {
    const source = '[:div "unterminated"';
    const h = html(() => renderBlocks(
      [{ kind: "hiccup", v: source }],
      undefined,
      undefined,
      false,
      format,
    ));
    expect(h).toContain(source);
    expect(h).not.toContain("<div>unterminated</div>");
  });

  it("raw block HTML preserves native audio and video elements", () => {
    const root = document.createElement("div");
    const dispose = render(() => renderBlocks([
      { kind: "raw_html", text: '<audio controls src="https://media.example/audio.ogg"></audio>' },
      { kind: "raw_html", text: '<video controls src="https://media.example/video.mp4"></video>' },
    ]), root);
    try {
      const audio = root.querySelector("audio");
      const video = root.querySelector("video");
      expect(audio).not.toBeNull();
      expect(audio?.hasAttribute("controls")).toBe(true);
      expect(audio?.getAttribute("src")).toBe("https://media.example/audio.ogg");
      expect(video).not.toBeNull();
      expect(video?.hasAttribute("controls")).toBe(true);
      expect(video?.getAttribute("src")).toBe("https://media.example/video.mp4");
    } finally {
      dispose();
    }
  });

  it("sanitizes hostile direct Hiccup through the shared insertion path", () => {
    const root = document.createElement("div");
    const dispose = render(() => renderBlocks([{
      kind: "hiccup",
      v: '[:div [:script "bad"] [:img {:src "javascript:bad()" :onerror "bad()"}] [:iframe {:src "https://evil.example"}]]',
    }]), root);
    try {
      expect(root.querySelector("script")).toBeNull();
      expect(root.querySelector("iframe")).toBeNull();
      expect(root.querySelector("[onerror]")).toBeNull();
      expect(root.innerHTML).not.toContain("javascript:");
      expect(root.innerHTML).not.toContain("bad()");
    } finally {
      dispose();
    }
  });

  // A `# heading` block's size applies ONLY to its first (heading) line — a `> quote`
  // continuation in the same block stays normal-size (OG parity, not h1). The heading
  // level is passed to renderBlocks; only block 0 is wrapped in heading-text.
  it("heading size wraps only the first block, not a continuation quote", () => {
    const h = html(() =>
      renderBlocks(
        [
          { kind: "paragraph", inline: [{ k: "plain", text: "Title" }] },
          { kind: "quote", children: [{ kind: "paragraph", inline: [{ k: "plain", text: "quoted" }] }] },
        ],
        undefined,
        1,
      ),
    );
    const headingSpan = /<span class="heading-text h1">.*?<\/span>/s.exec(h)?.[0] ?? "";
    expect(headingSpan).toContain("Title"); // heading line is h1
    expect(headingSpan).not.toContain("quoted"); // the quote is OUTSIDE the heading span
  });
});

describe("user macro helpers", () => {
  it("renders configured Hiccup as rich output through the main AstBody path", () => {
    setGraphMeta({ macros: { rich: '[:span {:class "lane-rich"} "Rich macro"]' } } as never);
    const h = html(() => <AstBody raw="{{rich}}" />);
    expect(h).toContain('class="lane-rich"');
    expect(h).toContain("Rich macro");
    expect(h).not.toContain("[:span");
  });

  it("threads macro Hiccup mode through block and nested quote rendering", () => {
    setGraphMeta({
      macros: {
        blocks: 'Intro\n[:div.block-rich "Block macro"]',
        quoted: '> [:span.quote-rich "Quote macro"]',
      },
    } as never);
    const h = html(() => <div><AstBody raw="{{blocks}}" /><AstBody raw="{{quoted}}" /></div>);
    expect(h).toContain('class="block-rich"');
    expect(h).toContain('class="quote-rich"');
    expect(h).toContain("Block macro");
    expect(h).toContain("Quote macro");
    expect(h).not.toContain("[:div.block-rich");
    expect(h).not.toContain("[:span.quote-rich");

    const nestedInline = html(() => renderInlines([{
      k: "emphasis",
      emph: "Bold",
      children: [{ k: "hiccup", v: '[:span.emph-rich "Emphasis macro"]' }],
    }], undefined, true, true));
    expect(nestedInline).toContain('class="emph-rich"');
    expect(nestedInline).not.toContain("[:span.emph-rich");
  });

  it("renders adjacent macro, direct inline, and direct block Hiccup through the shared path", () => {
    setGraphMeta({ macros: { rich: '[:span.macro-rich "Macro rich"]' } } as never);
    const h = html(() => (
      <div>
        {renderInlines([
          { k: "macro", name: "rich", args: [] },
          { k: "plain", text: " / " },
          { k: "hiccup", v: '[:span.direct-inline "Direct inline"]' },
        ])}
        <AstBody raw={'Intro\n[:div.direct-block "Direct block"]'} />
      </div>
    ));
    expect(h).toContain('class="macro-rich"');
    expect(h).toContain('class="direct-inline"');
    expect(h).toContain('class="direct-block"');
    expect(h).not.toContain("[:span.direct-inline");
    expect(h).not.toContain("[:div.direct-block");
  });

  it("keeps raw-HTML macro output sanitized", () => {
    setGraphMeta({
      macros: { raw: '<span class="raw-rich" onclick="alert(1)">Raw macro</span><script>bad()</script>' },
    } as never);
    const h = html(() => <AstBody raw="{{raw}}" />);
    expect(h).toContain('class="raw-rich"');
    expect(h).toContain("Raw macro");
    expect(h).not.toContain("onclick");
    expect(h).not.toContain("<script");
  });

  it("sanitizes hostile top-level and nested Hiccup without using the iframe fast-path", () => {
    setGraphMeta({
      macros: {
        script: '[:script "x"]',
        image: '[:img {:src "https://example.com/x.png" :onerror "x"}]',
        link: '[:a {:href "javascript:alert(1)"} "link"]',
        hostile: '[:div [:iframe {:src "https://example.com"} "nested frame"] [:span "<b>literal</b>"]]',
        frame: '[:iframe {:src "https://example.com"} "top frame"]',
        breakout: '[:span {:title "x\\" data-breakout=\\"yes><img src=\\"x"} "safe"]',
      },
    } as never);
    const root = document.createElement("div");
    const dispose = render(() => (
      <div>
        <AstBody raw="{{script}}" />
        <AstBody raw="{{image}}" />
        <AstBody raw="{{link}}" />
        <AstBody raw="{{hostile}}" />
        <AstBody raw="{{frame}}" />
        <AstBody raw="{{breakout}}" />
      </div>
    ), root);
    try {
      expect(root.querySelector("script")).toBeNull();
      expect(root.querySelector("iframe")).toBeNull();
      expect(root.querySelector("b")).toBeNull();
      expect(root.querySelector("[data-breakout]")).toBeNull();
      expect(root.querySelectorAll("img")).toHaveLength(1);
      expect(root.querySelector("[onerror]")).toBeNull();
      expect(root.querySelector("a")?.getAttribute("href")).toBeNull();
      expect(root.textContent).toContain("<b>literal</b>");
    } finally {
      dispose();
    }
  });

  it.each([
    '[:span',
    '(fn [] "x")',
    '[:span symbol]',
    '[:span #{"set"}]',
    '[:span #thing "tagged"]',
    '[:sp@n "bad"]',
  ])("falls back to literal macro text without crashing: %s", (source) => {
    setGraphMeta({ macros: { bad: source } } as never);
    const root = document.createElement("div");
    const dispose = render(() => <AstBody raw="{{bad}}" />, root);
    try {
      expect(root.textContent).toContain(source);
    } finally {
      dispose();
    }
  });

  it("still dispatches a query nested in a configured macro", async () => {
    vi.spyOn(backend(), "runQuery").mockResolvedValue([]);
    setGraphMeta({ root: "/test", macros: { outer: "{{query (task TODO)}}" } } as never);
    const root = document.createElement("div");
    const dispose = render(() => <AstBody raw="{{outer}}" />, root);
    try {
      await vi.waitFor(() => expect(backend().runQuery).toHaveBeenCalledWith("(task TODO)"));
    } finally {
      dispose();
    }
  });

  it("still renders nested configured macros and retains the recursion cap", () => {
    setGraphMeta({
      macros: {
        outer: "{{inner}}",
        inner: '[:strong.nested-rich "Nested macro"]',
        loop: "{{loop}}",
      },
    } as never);
    const nested = html(() => <AstBody raw="{{outer}}" />);
    expect(nested).toContain('class="nested-rich"');
    expect(nested).toContain("Nested macro");
    expect(() => html(() => <AstBody raw="{{loop}}" />)).not.toThrow();
    expect(html(() => <AstBody raw="{{loop}}" />)).toContain("{{loop}}");
  });

  it("leaves unfilled placeholders literal", () => {
    expect(expandTemplate("$1 and $5", ["a", "b"])).toBe("a and $5");
  });

  it("classifies block-level expansions through the block parser", () => {
    expect(expansionIsBlockLevel("## H\n\np\n\n- x", "md")).toBe(true);
    expect(expansionIsBlockLevel("## H", "md")).toBe(true);
    expect(expansionIsBlockLevel("just **bold** text", "md")).toBe(false);
  });

  it("uses lsdoc's parsed macro args instead of re-splitting quoted commas", () => {
    const parsed = parseBlock('{{m "a, b", c}}', false)[0];
    if (parsed.kind !== "bullet") throw new Error(`unexpected macro host: ${parsed.kind}`);
    const macro = parsed.inline.find((i) => i.k === "macro");
    expect(macro).toMatchObject({ k: "macro", name: "m", args: ['"a, b"', "c"] });

    setGraphMeta({ macros: { echo: "$1|$2" } } as never);
    const h = inl([{ k: "macro", name: "echo", args: ['"a, b"', "c"] }]);
    expect(h).toContain('"a, b"|c');
  });
});

// InlineText is the inline-only renderer for property values, breadcrumbs,
// ref-preview lines, etc. It parses via wasm; audit fix C1: a line that lsdoc
// parses as a BLOCK construct (and so has no inline-flow content) must fall back
// to the literal text, never render blank.
describe("InlineText", () => {
  const txt = (s: string, fmt?: "md" | "org") =>
    html(() => InlineText({ text: s, format: fmt }) as JSX.Element);
  // strip tags + un-escape the few entities the renderer emits, to read visible text
  const visible = (h: string) =>
    h.replace(/<[^>]*>/g, "").replace(/&gt;/g, ">").replace(/&lt;/g, "<").replace(/&amp;/g, "&");

  it("renders normal inline markup", () => {
    expect(txt("a **b**")).toContain("<strong>");
  });

  // Audit fix C4: the format prop must drive parsing, so org inline syntax renders
  // as org (italic), not literally, when the caller is on an org page.
  it("parses org inline markup as org when format=org (C4)", () => {
    expect(txt("/italic/", "org")).toContain("<em>");
    expect(txt("/italic/", "md")).not.toContain("<em>"); // md: literal slashes, no italic
  });

  it.each([
    ["> quote", "quote"],
    ["---", "---"],
    ["| a | b |", "a"],
    ["[^1]: def", "def"],
    ["$$E=mc^2$$", "mc"],
  ])("falls back to literal text for block-construct line %j (never blank)", (line, token) => {
    const text = visible(txt(line));
    expect(text.trim().length).toBeGreaterThan(0);
    expect(text).toContain(token);
  });
});

// A page reference shows the target page's `icon::` as a prefix (OG parity). The
// icon is fetched async + batched, so these await the resource resolving.
describe("inline page-icon prefix", () => {
  it("shows the referenced page's icon:: (Formula1 has 🏁 in the mock)", async () => {
    const div = document.createElement("div");
    const dispose = render(() => InlineText({ text: "[[Formula1]]" }) as JSX.Element, div);
    for (let i = 0; i < 60 && !div.innerHTML.includes("page-ref-icon"); i++) {
      await new Promise((r) => setTimeout(r, 5));
    }
    expect(div.innerHTML).toContain("page-ref-icon");
    expect(div.innerHTML).toContain("emoji"); // 🏁 → Twemoji <img class="emoji">
    dispose();
  });

  it("a page with no icon:: gets no prefix", async () => {
    const div = document.createElement("div");
    const dispose = render(() => InlineText({ text: "[[NoSuchIconPage]]" }) as JSX.Element, div);
    await new Promise((r) => setTimeout(r, 80));
    expect(div.innerHTML).not.toContain("page-ref-icon");
    dispose();
  });
});
