import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { clearSeededFacets } from "../render/facets";
import {
  blockProperty,
  doc,
  loadSingle,
  pageByName,
  pageToDto,
  resetStore,
} from "../store";
import { editingId, startEditing } from "../editorController";
import type { BlockDto, Format, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(() => initParser());

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function blk(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  return { id, raw, collapsed: false, children };
}

function page(blocks: BlockDto[], format: Format = "md"): PageDto {
  return { name: "Numbered", kind: "page", title: "Numbered", pre_block: null, blocks, format };
}

function load(blocks: BlockDto[], format: Format = "md"): void {
  loadSingle(page(blocks, format));
  // Hand-built DTOs omit the backend's parsed-property projection; exercise the
  // same derive-from-raw path used by store tests instead of seeding empty facets.
  clearSeededFacets();
}

function mountEditor(id: string): { textarea: HTMLTextAreaElement; dispose: () => void } {
  startEditing(id, 0);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(
    () => <For each={pageByName("Numbered")?.roots ?? []}>{(blockId) => <Block id={blockId} />}</For>,
    root,
  );
  return { textarea: root.querySelector("textarea.block-editor") as HTMLTextAreaElement, dispose };
}

function typeValue(textarea: HTMLTextAreaElement, value: string): void {
  textarea.focus();
  textarea.value = value;
  textarea.setSelectionRange(value.length, value.length);
  textarea.dispatchEvent(new InputEvent("input", {
    bubbles: true,
    inputType: "insertText",
    data: value.at(-1) ?? null,
  }));
}

function keydown(textarea: HTMLTextAreaElement, key: "Enter" | "Backspace", caret: number): KeyboardEvent {
  textarea.focus();
  textarea.setSelectionRange(caret, caret);
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  textarea.dispatchEvent(event);
  return event;
}

describe("own numbered-list editor invariant", () => {
  it("turns exactly '1. ' into the own property, clears text, and leaves the caret at zero", () => {
    load([blk("untouched", "untouched bytes"), blk("item", "")]);
    const { textarea, dispose } = mountEditor("item");
    try {
      typeValue(textarea, "1. ");
      expect(blockProperty("item", "logseq.order-list-type")).toBe("number");
      expect(textarea.value).toBe("");
      expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([0, 0]);
      expect(pageToDto("Numbered")!.blocks.find((block) => block.id === "untouched")!.raw).toBe("untouched bytes");
    } finally {
      dispose();
    }
  });

  it.each(["x1. ", "1. x"])("keeps non-exact trigger %j as literal text", (literal) => {
    load([blk("item", ""), blk("untouched", "untouched bytes")]);
    const { textarea, dispose } = mountEditor("item");
    try {
      typeValue(textarea, literal);
      expect(doc.byId.item.raw).toBe(literal);
      expect(textarea.value).toBe(literal);
      expect(blockProperty("item", "logseq.order-list-type")).toBeNull();
      expect(doc.byId.untouched.raw).toBe("untouched bytes");
    } finally {
      dispose();
    }
  });

  it("Enter on an empty non-nested own-numbered block stops the list without adding a block", () => {
    load([
      blk("untouched", "untouched bytes"),
      blk("item", "logseq.order-list-type:: number"),
    ]);
    const rootsBefore = [...pageByName("Numbered")!.roots];
    const { textarea, dispose } = mountEditor("item");
    try {
      const event = keydown(textarea, "Enter", 0);
      expect(event.defaultPrevented).toBe(true);
      expect(pageByName("Numbered")!.roots).toEqual(rootsBefore);
      expect(blockProperty("item", "logseq.order-list-type")).toBeNull();
      expect(editingId()).toBe("item");
      expect(doc.byId.untouched.raw).toBe("untouched bytes");
    } finally {
      dispose();
    }
  });

  it("Backspace at zero removes only the own-numbered property and keeps text and siblings exact", () => {
    load([
      blk("untouched", "untouched bytes"),
      blk("item", "text stays\nlogseq.order-list-type:: number"),
      blk("after", "after bytes"),
    ]);
    const { textarea, dispose } = mountEditor("item");
    try {
      const event = keydown(textarea, "Backspace", 0);
      expect(event.defaultPrevented).toBe(true);
      expect(doc.byId.item.raw).toBe("text stays");
      expect(textarea.value).toBe("text stays");
      expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([0, 0]);
      expect(doc.byId.untouched.raw).toBe("untouched bytes");
      expect(doc.byId.after.raw).toBe("after bytes");
    } finally {
      dispose();
    }
  });

  it("keeps the existing offset-zero merge behavior for a plain block", () => {
    load([blk("previous", "before "), blk("plain", "text")]);
    const { textarea, dispose } = mountEditor("plain");
    try {
      keydown(textarea, "Backspace", 0);
      expect(doc.byId.plain).toBeUndefined();
      expect(doc.byId.previous.raw).toBe("before text");
    } finally {
      dispose();
    }
  });

  it("preserves Tine's in-block Markdown ordered-list continuation and marker removal", () => {
    load([blk("item", "1. inner")]);
    const { textarea, dispose } = mountEditor("item");
    try {
      keydown(textarea, "Enter", textarea.value.length);
      expect(doc.byId.item.raw).toBe("1. inner\n2. ");
      keydown(textarea, "Backspace", textarea.value.length);
      expect(doc.byId.item.raw).toBe("1. inner\n");
      expect(blockProperty("item", "logseq.order-list-type")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("writes and removes the own property through an Org drawer", () => {
    load([blk("untouched", "untouched bytes"), blk("item", "")], "org");
    const { textarea, dispose } = mountEditor("item");
    try {
      typeValue(textarea, "1. ");
      expect(doc.byId.item.raw).toBe(":PROPERTIES:\n:logseq.order-list-type: number\n:END:");
      expect(textarea.value).toBe("");

      textarea.value = "org text";
      textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "t" }));
      keydown(textarea, "Backspace", 0);
      expect(doc.byId.item.raw).toBe("org text");
      expect(doc.byId.untouched.raw).toBe("untouched bytes");
    } finally {
      dispose();
    }
  });
});
