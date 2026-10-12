// GH #448 "Paste as link", through a mounted block editor (the command reads the
// clipboard itself and edits the focused block). Separate from
// blockKeyboardCommands.test.tsx because a bound page host reloads the fixture
// page out from under a mounted Block.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { startEditing } from "../editorController";
import { paletteCommands } from "../keybindings";
import { setToasts, toasts } from "../toasts";
import { Block } from "./Block";
import type { BlockDto } from "../types";

const A = "11111111-1111-4111-8111-111111111111";
const ref = "((cf1ef346-3c69-47f6-b09e-52b109ba6ee7))";
beforeAll(() => initParser());
afterEach(() => { resetStore(); setToasts([]); document.body.innerHTML = ""; });

function pasteAsLink(): void {
  const command = paletteCommands().find((c) => c.id === "editor/paste-as-link");
  expect(command, "the palette offers Paste as link").toBeDefined();
  command!.run();
}
function mountBlock(raw: string, start: number, end: number) {
  const dto: BlockDto = { id: A, raw: `${raw}\nid:: ${A}`, collapsed: false, children: [] };
  loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null, blocks: [dto] });
  startEditing(A, 0);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <For each={pageByName("P")?.roots ?? []}>{(id) => <Block id={id} />}</For>, root);
  const area = root.querySelector("textarea") as HTMLTextAreaElement;
  area.focus();
  area.setSelectionRange(start, end);
  return { area, dispose };
}
const clipboardHolds = (text: string) =>
  Object.defineProperty(navigator, "clipboard", { value: { readText: async () => text }, configurable: true });

describe("paste as link in the block editor (GH #448)", () => {
  it("turns the selected word into an aliased block ref when the clipboard holds a block ref", async () => {
    const { area, dispose } = mountBlock("see context here", 4, 11);
    try {
      clipboardHolds(ref);
      pasteAsLink();
      await vi.waitFor(() => expect(doc.byId[A].raw).toBe(`see [context](${ref}) here\nid:: ${A}`));
      expect(area.value).toBe(`see [context](${ref}) here`);
    } finally {
      dispose();
    }
  });

  it("wraps the selection around a copied URL", async () => {
    const { area, dispose } = mountBlock("see context here", 4, 11);
    try {
      clipboardHolds("https://example.org/a");
      pasteAsLink();
      await vi.waitFor(() => expect(area.value).toBe("see [context](https://example.org/a) here"));
    } finally {
      dispose();
    }
  });

  it("tells the user to select text when no block is being edited", () => {
    pasteAsLink();
    expect(toasts().some((t) => t.message.includes("Paste as link"))).toBe(true);
  });

  it("leaves the block alone and says so when the clipboard holds no link", async () => {
    const { area, dispose } = mountBlock("see context here", 4, 11);
    try {
      clipboardHolds("plain words");
      pasteAsLink();
      await vi.waitFor(() => expect(toasts().some((t) => t.message.includes("doesn't hold a link"))).toBe(true));
      expect(area.value).toBe("see context here");
      expect(doc.byId[A].raw).toBe(`see context here\nid:: ${A}`);
    } finally {
      dispose();
    }
  });
});
