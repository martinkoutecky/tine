import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore, undo } from "../store";
import { startEditing } from "../editorController";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(() => initParser());
afterEach(() => { resetStore(); document.body.innerHTML = ""; });

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function paste(textarea: HTMLTextAreaElement, text: string, html = ""): Event {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", {
    value: {
      files: [],
      items: [],
      types: html ? ["text/plain", "text/html"] : ["text/plain"],
      getData: (type: string) => type === "text/html" ? html : text,
    },
  });
  textarea.dispatchEvent(event);
  return event;
}

function keydown(textarea: HTMLTextAreaElement, init: KeyboardEventInit) {
  textarea.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
}

describe("multiline paste into editor-visible empty blocks", () => {
  it("replaces an id-only host instead of leaving a ghost blank bullet", () => {
    const block: BlockDto = {
      id: "11111111-1111-4111-8111-111111111111",
      raw: "id:: 11111111-1111-4111-8111-111111111111",
      collapsed: false,
      children: [],
    };
    const page: PageDto = { name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] };
    loadSingle(page);
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      expect(textarea.value).toBe("");
      paste(textarea, "- first\n- second");
      expect(pageByName("Paste")!.roots).toHaveLength(2);
      expect(pageByName("Paste")!.roots[0]).toBe(block.id);
      expect(doc.byId[block.id].raw).toBe("first\nid:: 11111111-1111-4111-8111-111111111111");
      expect(doc.byId[pageByName("Paste")!.roots[1]].raw).toBe("second");
    } finally {
      dispose();
    }
  });

  it("keeps Ctrl+Shift+V multiline plain text inside the current block", () => {
    const block: BlockDto = {
      id: "22222222-2222-4222-8222-222222222222",
      raw: "before after",
      collapsed: false,
      children: [],
    };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 7);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      textarea.setSelectionRange(7, 7);
      keydown(textarea, { key: "v", code: "KeyV", ctrlKey: true, shiftKey: true });
      const event = paste(textarea, "first\nsecond");

      expect(event.defaultPrevented).toBe(true);
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe("before first\nsecondafter");
    } finally {
      dispose();
    }
  });

  it("does not leak an aborted Ctrl+Shift+V gesture into a later paste", () => {
    const block: BlockDto = { id: "33333333-3333-4333-8333-333333333333", raw: "", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      keydown(textarea, { key: "v", code: "KeyV", ctrlKey: true, shiftKey: true });
      textarea.dispatchEvent(new KeyboardEvent("keyup", { key: "v", code: "KeyV", bubbles: true }));
      paste(textarea, "first\nsecond");
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe("first\nsecond");
    } finally {
      dispose();
    }
  });

  it("keeps plain prose with single newlines inside the current selection", async () => {
    const block: BlockDto = {
      id: "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
      raw: "πrefix SELECT suffix🧪",
      collapsed: false,
      children: [],
    };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      const start = textarea.value.indexOf("SELECT");
      textarea.setSelectionRange(start, start + "SELECT".length);
      const plain = "line1\nline2\nline3";
      const event = paste(textarea, plain);
      await Promise.resolve();

      expect(event.defaultPrevented).toBe(true);
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe(`πrefix ${plain} suffix🧪`);
      expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([
        start + plain.length,
        start + plain.length,
      ]);
    } finally {
      dispose();
    }
  });

  it("turns blank-line-separated plain paragraphs into trimmed blocks", () => {
    const block: BlockDto = { id: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee", raw: "", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      paste(root.querySelector("textarea") as HTMLTextAreaElement, "  a\n\nb\n\n\n c  ");
      expect(pageByName("Paste")!.roots.map((id) => doc.byId[id].raw)).toEqual(["a", "b", "c"]);
    } finally {
      dispose();
    }
  });

  it("keeps indented unbulleted lines literal inside the current block", () => {
    const block: BlockDto = { id: "ffffffff-ffff-4fff-8fff-ffffffffffff", raw: "before after", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 7);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      textarea.setSelectionRange(7, 7);
      paste(textarea, "  indented\n    deeper");
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe("before   indented\n    deeperafter");
    } finally {
      dispose();
    }
  });

  it("parses Markdown ATX headings as blocks", () => {
    const block: BlockDto = { id: "12121212-1212-4212-8212-121212121212", raw: "", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      paste(root.querySelector("textarea") as HTMLTextAreaElement, "# heading\nbody");
      expect(pageByName("Paste")!.roots.map((id) => doc.byId[id].raw)).toEqual(["# heading", "body"]);
    } finally {
      dispose();
    }
  });

  it("does not treat numbered Markdown lines as block-looking text", () => {
    const block: BlockDto = { id: "13131313-1313-4313-8313-131313131313", raw: "", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const numbered = "1. first\n2. second";
      paste(root.querySelector("textarea") as HTMLTextAreaElement, numbered);
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe(numbered);
    } finally {
      dispose();
    }
  });

  it("uses Org star detection while keeping Org prose literal", () => {
    const headingBlock: BlockDto = { id: "14141414-1414-4414-8414-141414141414", raw: "", collapsed: false, children: [] };
    loadSingle({
      name: "Org Paste",
      kind: "page",
      title: "Org Paste",
      pre_block: null,
      format: "org",
      blocks: [headingBlock],
    });
    startEditing(headingBlock.id, 0);
    const first = mount(() => (
      <For each={pageByName("Org Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      paste(first.root.querySelector("textarea") as HTMLTextAreaElement, "* first\n* second");
      expect(pageByName("Org Paste")!.roots.map((id) => doc.byId[id].raw)).toEqual(["first", "second"]);
    } finally {
      first.dispose();
    }

    resetStore();
    document.body.innerHTML = "";
    const proseBlock: BlockDto = { id: "15151515-1515-4515-8515-151515151515", raw: "", collapsed: false, children: [] };
    loadSingle({
      name: "Org Prose",
      kind: "page",
      title: "Org Prose",
      pre_block: null,
      format: "org",
      blocks: [proseBlock],
    });
    startEditing(proseBlock.id, 0);
    const second = mount(() => (
      <For each={pageByName("Org Prose")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const prose = "line one\nline two";
      paste(second.root.querySelector("textarea") as HTMLTextAreaElement, prose);
      expect(pageByName("Org Prose")!.roots).toEqual([proseBlock.id]);
      expect(doc.byId[proseBlock.id].raw).toBe(prose);
    } finally {
      second.dispose();
    }
  });

  it("atomically replaces an empty block with a structured HTML outline", () => {
    const block: BlockDto = { id: "44444444-4444-4444-8444-444444444444", raw: "", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      paste(textarea, "Parent\nChild\nSibling", "<ul><li>Parent<ul><li>Child</li></ul></li><li>Sibling</li></ul>");
      const roots = pageByName("Paste")!.roots;
      expect(roots).toHaveLength(2);
      expect(doc.byId[roots[0]].raw).toBe("Parent");
      expect(doc.byId[doc.byId[roots[0]].children[0]].raw).toBe("Child");
      expect(doc.byId[roots[1]].raw).toBe("Sibling");

      undo();
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe("");
    } finally {
      dispose();
    }
  });

  it("keeps a Markdown table source in one block", () => {
    const block: BlockDto = { id: "55555555-5555-4555-8555-555555555555", raw: "", collapsed: false, children: [] };
    const table = "| Name | Value |\n| --- | ---: |\n| Alpha | 1 |";
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      paste(root.querySelector("textarea") as HTMLTextAreaElement, table);
      const roots = pageByName("Paste")!.roots;
      expect(roots).toHaveLength(1);
      expect(doc.byId[roots[0]].raw).toBe(table);
    } finally {
      dispose();
    }
  });

  it("uses plain text when Ctrl+Shift+V accompanies structured HTML", () => {
    const block: BlockDto = { id: "66666666-6666-4666-8666-666666666666", raw: "before", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 6);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      textarea.setSelectionRange(6, 6);
      keydown(textarea, { key: "v", code: "KeyV", ctrlKey: true, shiftKey: true });
      paste(textarea, "\nParent\nChild", "<ul><li>Parent<ul><li>Child</li></ul></li></ul>");
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe("before\nParent\nChild");
    } finally {
      dispose();
    }
  });

  it("raw-pastes single-line rich clipboard text byte-for-byte over a selection", async () => {
    const block: BlockDto = {
      id: "77777777-7777-4777-8777-777777777777",
      raw: "πrefix SELECT suffix🧪",
      collapsed: false,
      children: [],
    };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      const start = textarea.value.indexOf("SELECT");
      textarea.setSelectionRange(start, start + "SELECT".length);
      keydown(textarea, { key: "v", code: "KeyV", ctrlKey: true, shiftKey: true });
      const plain = "literal [plain](https://plain.test)";
      const event = paste(textarea, plain, "<ul><li>HTML</li><li>Outline</li></ul>");
      await Promise.resolve();

      expect(event.defaultPrevented).toBe(true);
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe(`πrefix ${plain} suffix🧪`);
      expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([
        start + plain.length,
        start + plain.length,
      ]);
    } finally {
      dispose();
    }
  });

  it("raw-pastes a selected bare URL literally instead of wrapping a link", async () => {
    const block: BlockDto = {
      id: "88888888-8888-4888-8888-888888888888",
      raw: "before selected after",
      collapsed: false,
      children: [],
    };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      const start = textarea.value.indexOf("selected");
      textarea.setSelectionRange(start, start + "selected".length);
      keydown(textarea, { key: "v", code: "KeyV", metaKey: true, shiftKey: true });
      const url = "https://youtu.be/dQw4w9WgXcQ";
      const event = paste(textarea, url);
      await Promise.resolve();

      expect(event.defaultPrevented).toBe(true);
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe(`before ${url} after`);
      expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([
        start + url.length,
        start + url.length,
      ]);
    } finally {
      dispose();
    }
  });

  it("claims an empty raw clipboard flavor without deleting the selection", () => {
    const block: BlockDto = {
      id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
      raw: "before selected after",
      collapsed: false,
      children: [],
    };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      const start = textarea.value.indexOf("selected");
      textarea.setSelectionRange(start, start + "selected".length);
      keydown(textarea, { key: "v", code: "KeyV", ctrlKey: true, shiftKey: true });
      const event = paste(textarea, "", "<p>HTML fallback must not run</p>");

      expect(event.defaultPrevented).toBe(true);
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe("before selected after");
    } finally {
      dispose();
    }
  });

  it.each([
    "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
    "https://y2u.be/dQw4w9WgXcQ",
    "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
    "https://www.loom.com/share/1234-abcd",
    "https://player.vimeo.com/video/123456789",
    "https://www.bilibili.com/video/BV1xx411c7mD",
  ])("normalizes a bare OG video-host URL as {{video URL}}: %s", (url) => {
    const block: BlockDto = {
      id: "99999999-9999-4999-8999-999999999999",
      raw: "before ",
      collapsed: false,
      children: [],
    };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, block.raw.length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      textarea.setSelectionRange(textarea.value.length, textarea.value.length);
      const event = paste(textarea, url);

      expect(event.defaultPrevented).toBe(true);
      expect(doc.byId[block.id].raw).toBe(`before {{video ${url}}}`);
    } finally {
      dispose();
    }
  });

  it("keeps selected-URL wrapping ahead of video normalization", () => {
    const block: BlockDto = {
      id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      raw: "before selected after",
      collapsed: false,
      children: [],
    };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      const start = textarea.value.indexOf("selected");
      textarea.setSelectionRange(start, start + "selected".length);
      const url = "https://youtu.be/dQw4w9WgXcQ";
      const event = paste(textarea, url);

      expect(event.defaultPrevented).toBe(true);
      expect(doc.byId[block.id].raw).toBe(`before [selected](${url}) after`);
    } finally {
      dispose();
    }
  });

  it("leaves a non-video bare URL to the existing native paste path", () => {
    const block: BlockDto = {
      id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
      raw: "before ",
      collapsed: false,
      children: [],
    };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, block.raw.length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      textarea.setSelectionRange(textarea.value.length, textarea.value.length);
      const event = paste(textarea, "https://example.com/watch?v=dQw4w9WgXcQ");

      expect(event.defaultPrevented).toBe(false);
      expect(doc.byId[block.id].raw).toBe("before ");
    } finally {
      dispose();
    }
  });
});
