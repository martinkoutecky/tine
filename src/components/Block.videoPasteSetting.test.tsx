import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { startEditing } from "../editorController";
import { setAutoEmbedVideoLinks } from "../pasteSettings";
import { initParser } from "../render/parse";
import { editorIn, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { Block } from "./Block";

const ID = "86100000-1111-4111-8111-111111111111";
const providers = [
  "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
  "https://youtu.be/dQw4w9WgXcQ",
  "https://vimeo.com/76979871",
  "https://loom.com/share/abc123",
  "https://www.bilibili.com/video/BV1xx411c7mD",
];

beforeAll(() => initParser());
beforeEach(() => vi.spyOn(backend(), "setAppBool").mockResolvedValue());
afterEach(async () => {
  setAutoEmbedVideoLinks(true);
  await new Promise(resolve => setTimeout(resolve, 0));
  vi.restoreAllMocks(); resetStore(); document.body.replaceChildren();
});

describe.each<EditorKind>(["textarea", "cm6"])("GH #861 pasted video links (%s)", kind => {
  useEditorKind(kind);

  function mount(format: "md" | "org" = "md", raw = "before  after") {
    loadSingle({ name: "Video links", kind: "page", title: "Video links", format,
      pre_block: null, blocks: [{ id: ID, raw, collapsed: false, children: [] }] });
    startEditing(ID, 7);
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <For each={pageByName("Video links")?.roots ?? []}>{id => <Block id={id} />}</For>, root);
    const editor = editorIn(root); expect(editor.kind).toBe(kind);
    editor.focus(); editor.select(7, 7);
    return { editor, dispose };
  }

  it.each(providers.flatMap(url => [true, false].map(on => ({ url, on }))))(
    "pastes $url with auto-embed=$on", async ({ url, on }) => {
      setAutoEmbedVideoLinks(on);
      const { editor, dispose } = mount();
      try {
        const event = editor.paste({ text: url });
        // jsdom has no browser default insertion for textareas. Model it only
        // when the real paste handler leaves the event to the browser.
        if (kind === "textarea" && !event.defaultPrevented) editor.input(`before ${url} after`, 7 + url.length);
        const inserted = on ? `{{video ${url}}}` : url;
        expect(editor.value()).toBe(`before ${inserted} after`);
        await vi.waitFor(() => expect(editor.selection()).toEqual([7 + inserted.length, 7 + inserted.length]));
        await vi.waitFor(() => expect(doc.byId[ID].raw).toBe(`before ${inserted} after`));
      } finally { dispose(); }
    });

  it.each(["md", "org"] as const)("keeps linking selected words in %s when auto-embed is off", format => {
    setAutoEmbedVideoLinks(false);
    const { editor, dispose } = mount(format, "before context after");
    try {
      editor.select(7, 14);
      editor.paste({ text: providers[0] });
      const link = format === "org" ? `[[${providers[0]}][context]]` : `[context](${providers[0]})`;
      expect(editor.value()).toBe(`before ${link} after`);
    } finally { dispose(); }
  });

  it("honors a changed preference in an already open editor", async () => {
    const { editor, dispose } = mount();
    try {
      setAutoEmbedVideoLinks(false);
      const event = editor.paste({ text: providers[0] });
      if (kind === "textarea" && !event.defaultPrevented) editor.input(`before ${providers[0]} after`, 7 + providers[0].length);
      await Promise.resolve();
      setAutoEmbedVideoLinks(true);
      editor.paste({ text: providers[1] });
      expect(editor.value()).toBe(`before ${providers[0]}{{video ${providers[1]}}} after`);
    } finally { dispose(); }
  });
});
